// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! macOS: retained `IOSurfaceRef` -> `MTLTexture` -> Graft -> wgpu Metal.
//!
//! Welding owns the CEF/IOSurface lifetime and texture descriptor. Graft owns
//! the backend-specific `MTLTexture` -> `wgpu::Texture` wrapper.

use super::*;

/// The Apple half of [`WgpuTextureImporter::import`] for
/// [`NativeFrame::MetalTextureRef`] frames.
pub(super) fn import_metal(
    frame: MetalTextureRef,
    ctx: &HostWgpuContext,
) -> Result<ImportedTexture, ImportError> {
    use objc2_io_surface::IOSurfaceRef;
    use objc2_metal::{MTLDevice, MTLStorageMode, MTLTextureDescriptor, MTLTextureUsage};

    if frame.io_surface.is_null() {
        return Err(ImportError::InvalidFrame("IOSurface handle is null"));
    }
    if frame.size.width == 0 || frame.size.height == 0 {
        return Err(ImportError::InvalidFrame("Metal IOSurface has zero size"));
    }
    if ctx.backend != InteropBackend::Metal {
        return Err(ImportError::BackendMismatch {
            frame: NativeFrameKind::MetalTextureRef,
            wgpu: ctx.backend,
        });
    }

    let pixel_format = wgpu_format_to_mtl(frame.format).ok_or_else(|| {
        ImportError::MetalImport(format!(
            "unsupported wgpu format for Metal IOSurface import: {:?}",
            frame.format
        ))
    })?;

    let mtl_texture = unsafe {
        let mtl_device =
            crate::wgpu_compat::metal_device(&ctx.device).ok_or(ImportError::BackendMismatch {
                frame: NativeFrameKind::MetalTextureRef,
                wgpu: ctx.backend,
            })?;

        // `newTextureWithDescriptor:iosurface:plane:` takes the CoreFoundation
        // `IOSurfaceRef`, not the ObjC `IOSurface` class, and CEF hands us the
        // retained CF pointer.
        let io_surface = &*(frame.io_surface as *const IOSurfaceRef);
        let descriptor =
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                pixel_format,
                frame.size.width as usize,
                frame.size.height as usize,
                false,
            );
        descriptor.setStorageMode(MTLStorageMode::Shared);
        descriptor.setUsage(MTLTextureUsage::ShaderRead);

        mtl_device
            .newTextureWithDescriptor_iosurface_plane(&descriptor, io_surface, 0)
            .ok_or_else(|| {
                ImportError::MetalImport(
                    "MTLDevice::newTextureWithDescriptor:iosurface:plane: returned nil".into(),
                )
            })?
    };

    let graft_host = grafting::HostWgpuContext::new(ctx.device.clone(), ctx.queue.clone());
    let graft_frame = grafting::MetalTextureRef::new(
        grafting::FrameMetadata {
            size: frame.size,
            format: frame.format,
            generation: frame.generation,
            producer_sync: grafting::SyncMechanism::None,
        },
        mtl_texture,
    );
    let texture = grafting::import_metal_texture_ref(graft_frame, &graft_host)
        .map_err(|error| ImportError::MetalImport(error.to_string()))?;

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(ImportedTexture {
        texture,
        view,
        size: wgpu::Extent3d {
            width: frame.size.width,
            height: frame.size.height,
            depth_or_array_layers: 1,
        },
        format: frame.format,
        generation: frame.generation,
    })
}

fn wgpu_format_to_mtl(format: wgpu::TextureFormat) -> Option<objc2_metal::MTLPixelFormat> {
    use objc2_metal::MTLPixelFormat;
    match format {
        wgpu::TextureFormat::Bgra8Unorm => Some(MTLPixelFormat::BGRA8Unorm),
        wgpu::TextureFormat::Rgba8Unorm => Some(MTLPixelFormat::RGBA8Unorm),
        // wubble change: the sRGB frame copy (`copy_metal_to_owned`).
        wgpu::TextureFormat::Bgra8UnormSrgb => Some(MTLPixelFormat::BGRA8Unorm_sRGB),
        wgpu::TextureFormat::Rgba8UnormSrgb => Some(MTLPixelFormat::RGBA8Unorm_sRGB),
        _ => None,
    }
}

/// wubble change: copies a callback-scoped CEF IOSurface into a weld-owned
/// texture before returning. CEF renders into pooled IOSurfaces: the
/// `CFRetain` keeps the object alive but not its contents, so a deferred
/// import could read a newer frame. cef-rs does the same copy
/// (`cef/src/osr_texture_import/iosurface.rs`, `import_to_wgpu` then
/// `texture::copy_to_owned`), MIT OR Apache-2.0.
///
/// Unlike the shared [`super::copy_to_owned`], the blit runs on a Metal
/// command queue of its own and only that command buffer is waited for: on
/// wgpu's queue the copy waited behind the scene and Slint's frame already
/// submitted there (1.2 to 2.8 ms median, 5 to 8 ms at p95/p99 on an M2 Pro,
/// with the UI thread blocked). The waited-for copy is complete before wgpu
/// records any use of it, so the two queues need no other synchronisation.
/// The copy is created as a raw `MTLTexture` and wrapped like the import, so
/// wgpu counts it as initialised and never clears it.
///
/// wubble change (64): with `pool` (the host asked for OS buffers) the copy goes into one of
/// its IOSurfaces instead, carried out as [`CopiedFrame::os`]; `Ok(None)` when every one is in
/// use (the frame is dropped). Without it this path is as before.
pub(crate) fn copy_metal_to_owned(
    frame: MetalTextureRef,
    ctx: &HostWgpuContext,
    arrived: std::time::Instant,
    pool: Option<&mut OsBufferPool>,
) -> Result<Option<CopiedFrame>, ImportError> {
    use objc2_metal::{
        MTLBlitCommandEncoder, MTLCommandBuffer, MTLCommandBufferStatus, MTLCommandEncoder,
        MTLCommandQueue, MTLDevice, MTLStorageMode, MTLTextureDescriptor, MTLTextureUsage,
    };

    let started = std::time::Instant::now();
    let format = frame.format.add_srgb_suffix();
    let size = frame.size;
    let generation = frame.generation;
    let pixel_format = wgpu_format_to_mtl(format).ok_or_else(|| {
        ImportError::MetalImport(format!("unsupported wgpu format for the frame copy: {format:?}"))
    })?;
    let source = import_metal(frame, ctx)?;
    // Retained out of the HAL guard: wgpu calls below would take its lock again.
    let source_raw = unsafe { source.texture.as_hal::<wgpu::wgc::api::Metal>() }
        .map(|t| objc2::Message::retain(t.raw_handle()))
        .ok_or_else(|| ImportError::MetalImport("the import is not a Metal texture".into()))?;
    let mtl_device = unsafe { crate::wgpu_compat::metal_device(&ctx.device) }.ok_or(
        ImportError::BackendMismatch {
            frame: NativeFrameKind::MetalTextureRef,
            wgpu: ctx.backend,
        },
    )?;
    // wubble change (64): the host's OS buffer, if it asked for them and one is free.
    let os = match pool.map(|pool| {
        pool.next(size, format, || Slot::new(&mtl_device, size, format, pixel_format))
    }) {
        None | Some(Next::Unavailable) => None,
        Some(Next::Buffer(buffer)) => Some(buffer),
        Some(Next::Busy) => return Ok(None),
    };
    let copy = match &os {
        Some(buffer) => Some(buffer.0.texture.clone()),
        None => unsafe {
            let descriptor =
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    pixel_format,
                    size.width as usize,
                    size.height as usize,
                    false,
                );
            descriptor.setStorageMode(MTLStorageMode::Private);
            descriptor.setUsage(MTLTextureUsage::ShaderRead);
            mtl_device.newTextureWithDescriptor(&descriptor)
        },
    }
    .ok_or_else(|| ImportError::MetalImport("newTextureWithDescriptor: returned nil".into()))?;

    let queue = copy_queue(&mtl_device)?;
    let buffer = queue
        .commandBuffer()
        .ok_or_else(|| ImportError::MetalImport("commandBuffer returned nil".into()))?;
    let blit = buffer
        .blitCommandEncoder()
        .ok_or_else(|| ImportError::MetalImport("blitCommandEncoder returned nil".into()))?;
    unsafe { blit.copyFromTexture_toTexture(&source_raw, &copy) };
    blit.endEncoding();
    buffer.commit();
    let waiting = std::time::Instant::now();
    buffer.waitUntilCompleted();
    let done = std::time::Instant::now();
    if buffer.status() != MTLCommandBufferStatus::Completed {
        return Err(ImportError::MetalImport(format!(
            "the frame copy did not complete: {:?}",
            buffer.status()
        )));
    }

    let graft_host = grafting::HostWgpuContext::new(ctx.device.clone(), ctx.queue.clone());
    let graft_frame = grafting::MetalTextureRef::new(
        grafting::FrameMetadata {
            size,
            format,
            generation,
            producer_sync: grafting::SyncMechanism::None,
        },
        copy,
    );
    let texture = grafting::import_metal_texture_ref(graft_frame, &graft_host)
        .map_err(|error| ImportError::MetalImport(error.to_string()))?;
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(Some(CopiedFrame {
        texture: ImportedTexture {
            texture,
            view,
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            format,
            generation,
        },
        arrived,
        copy: done - started,
        copy_wait: done - waiting,
        damage: None,
        os,
    }))
}

type Retained<T> = objc2::rc::Retained<objc2::runtime::ProtocolObject<T>>;

thread_local! {
    /// wubble change: the frame copies' command queue, per device (CEF calls
    /// `OnAcceleratedPaint` on its UI thread only).
    static COPY_QUEUE: std::cell::RefCell<Option<(
        Retained<dyn objc2_metal::MTLDevice>,
        Retained<dyn objc2_metal::MTLCommandQueue>,
    )>> = const { std::cell::RefCell::new(None) };
}

/// wubble change: the copy queue for `device`, made on first use.
fn copy_queue(
    device: &Retained<dyn objc2_metal::MTLDevice>,
) -> Result<Retained<dyn objc2_metal::MTLCommandQueue>, ImportError> {
    use objc2_metal::MTLDevice;
    COPY_QUEUE.with_borrow_mut(|cached| {
        if let Some((d, q)) = cached.as_ref()
            && objc2::rc::Retained::as_ptr(d) == objc2::rc::Retained::as_ptr(device)
        {
            return Ok(q.clone());
        }
        let queue = device
            .newCommandQueue()
            .ok_or_else(|| ImportError::MetalImport("newCommandQueue returned nil".into()))?;
        *cached = Some((device.clone(), queue.clone()));
        Ok(queue)
    })
}

/// wubble change (64): one OS buffer of a view's pool: an IOSurface and the Metal texture over
/// it that the frame copy writes.
pub(crate) struct Slot {
    surface: objc2_core_foundation::CFRetained<objc2_io_surface::IOSurfaceRef>,
    texture: Retained<dyn objc2_metal::MTLTexture>,
}

// SAFETY: an IOSurface and a Metal texture may be used from any thread (IOSurface and Metal
// objects are thread safe); the slot holds nothing else.
unsafe impl Send for Slot {}
unsafe impl Sync for Slot {}

impl Slot {
    /// A `size` IOSurface of `format`'s layout (BGRA or RGBA, 4 bytes a pixel) tagged sRGB, as
    /// CEF's bytes are display-encoded sRGB and the shell's layer is sRGB too, and its texture
    /// as `pixel_format` (the copy's sRGB format).
    fn new(
        device: &Retained<dyn objc2_metal::MTLDevice>,
        size: dpi::PhysicalSize<u32>,
        format: wgpu::TextureFormat,
        pixel_format: objc2_metal::MTLPixelFormat,
    ) -> Result<Self, ImportError> {
        use objc2_core_foundation::{CFDictionary, CFNumber, CFString, CFType};
        use objc2_io_surface::{
            IOSurfaceRef, kIOSurfaceBytesPerElement, kIOSurfaceBytesPerRow, kIOSurfaceColorSpace,
            kIOSurfaceHeight, kIOSurfacePixelFormat, kIOSurfaceWidth,
        };
        use objc2_metal::{MTLDevice, MTLStorageMode, MTLTextureDescriptor, MTLTextureUsage};

        let fourcc: &[u8; 4] = match format {
            wgpu::TextureFormat::Bgra8UnormSrgb => b"BGRA",
            wgpu::TextureFormat::Rgba8UnormSrgb => b"RGBA",
            _ => {
                return Err(ImportError::MetalImport(format!(
                    "no IOSurface layout for {format:?}"
                )));
            }
        };
        let (width, height) = (size.width as usize, size.height as usize);
        // SAFETY: IOSurface's own constant keys.
        let (keys, color_space) = unsafe {
            (
                [
                    kIOSurfaceWidth,
                    kIOSurfaceHeight,
                    kIOSurfaceBytesPerElement,
                    kIOSurfaceBytesPerRow,
                    kIOSurfacePixelFormat,
                ],
                kIOSurfaceColorSpace,
            )
        };
        let row = IOSurfaceRef::align_property(keys[3], width * 4);
        let (w, h) = (CFNumber::new_i64(width as i64), CFNumber::new_i64(height as i64));
        let (bytes, stride) = (CFNumber::new_i64(4), CFNumber::new_i64(row as i64));
        let layout = CFNumber::new_i32(u32::from_be_bytes(*fourcc) as i32);
        let values: [&CFType; 5] = [&w, &h, &bytes, &stride, &layout];
        let properties = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);
        // SAFETY: the dictionary holds IOSurface's keys with values of their types.
        let surface = unsafe { IOSurfaceRef::new(properties.as_opaque()) }
            .ok_or_else(|| ImportError::MetalImport("IOSurfaceCreate returned NULL".into()))?;
        // The name CoreGraphics' `kCGColorSpaceSRGB` holds, as Chromium tags its IOSurfaces
        // (`ui/gfx/mac/io_surface.cc`, `IOSurfaceSetColorSpace`).
        let srgb = CFString::from_static_str("kCGColorSpaceSRGB");
        // SAFETY: the colour space key takes a colour space name.
        unsafe { surface.set_value(color_space, &srgb) };

        let texture = unsafe {
            let descriptor =
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    pixel_format,
                    width,
                    height,
                    false,
                );
            descriptor.setStorageMode(MTLStorageMode::Shared);
            descriptor.setUsage(MTLTextureUsage::ShaderRead);
            device.newTextureWithDescriptor_iosurface_plane(&descriptor, &surface, 0)
        }
        .ok_or_else(|| {
            ImportError::MetalImport(
                "newTextureWithDescriptor:iosurface:plane: returned nil for an OS buffer".into(),
            )
        })?;
        Ok(Self { surface, texture })
    }

    pub(crate) fn io_surface(&self) -> std::ptr::NonNull<std::ffi::c_void> {
        std::ptr::NonNull::from(&*self.surface).cast()
    }

    /// The compositor uses it (`IOSurfaceIsInUse`); the shared pool also counts live buffers.
    pub(crate) fn compositor_uses(&self) -> bool {
        self.surface.is_in_use()
    }
}
