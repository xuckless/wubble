// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! wubble change (71): Linux OS buffers. One slot of a view's pool is a Vulkan image on the
//! host's device, made with a DRM format modifier the compositor takes
//! (`VK_EXT_image_drm_format_modifier`) and exported as one dmabuf plane
//! (`VK_KHR_external_memory_fd`), imported into wgpu once as the frame copy's destination and
//! the presenter's source. The host wraps the dmabuf in a `wl_buffer` (Wayland's
//! `zwp_linux_dmabuf_v1`) and tells the slot while the compositor holds it.
//!
//! The image making is the one wubble's `layer_probe` example ran on NVIDIA under mutter
//! (wubble, docs/research/linux-page-layer.md section 7: block-linear modifiers accepted, no
//! foreign-queue release barrier needed there). The copy into a slot is waited for on the CPU
//! before the frame is handed over, so the compositor never reads a half-written frame.

use std::os::fd::{AsFd, BorrowedFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ash::vk;

use super::{ImportError, OsBuffer};
use dpi::PhysicalSize;

/// `DRM_FORMAT_MOD_LINEAR` from `drm_fourcc.h`.
const LINEAR: u64 = 0;

const fn fourcc(c: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*c)
}

/// A slot's formats by the frame copy's format: (wgpu format, DRM fourcc, Vulkan format). CEF's
/// bytes are display-encoded sRGB and sampled through an sRGB view; the compositor reads the
/// same bytes as its fourcc. X (no alpha): a page is opaque, so the compositor may skip blending.
const SLOT_FORMATS: &[(wgpu::TextureFormat, u32, vk::Format)] = &[
    (wgpu::TextureFormat::Bgra8UnormSrgb, fourcc(b"XR24"), vk::Format::B8G8R8A8_SRGB),
    (wgpu::TextureFormat::Rgba8UnormSrgb, fourcc(b"XB24"), vk::Format::R8G8B8A8_SRGB),
];

/// The modifiers a slot of `drm` may be made with: the driver's single-plane ones (`driver`:
/// modifier, plane count, its tiling features) that can be copied into and sampled, and that
/// the compositor `offered` for `drm`; LINEAR when none is common (the compositor may still
/// refuse it, which the host hears as a failed buffer).
fn slot_modifiers(
    driver: &[(u64, u32, vk::FormatFeatureFlags)],
    offered: &[(u32, u64)],
    drm: u32,
) -> Vec<u64> {
    let needed = vk::FormatFeatureFlags::TRANSFER_DST | vk::FormatFeatureFlags::SAMPLED_IMAGE;
    let common: Vec<u64> = driver
        .iter()
        .filter(|&&(modifier, planes, features)| {
            planes == 1 && features.contains(needed) && offered.contains(&(drm, modifier))
        })
        .map(|&(modifier, ..)| modifier)
        .collect();
    if common.is_empty() { vec![LINEAR] } else { common }
}

/// One OS buffer: the exported image, as a wgpu texture and as a dmabuf.
pub(crate) struct Slot {
    texture: wgpu::Texture,
    fd: OwnedFd,
    offset: u32,
    stride: u32,
    modifier: u64,
    drm_format: u32,
    size: PhysicalSize<u32>,
    /// The compositor holds it: set by the host when it attaches the buffer, cleared on the
    /// compositor's `wl_buffer.release` (or by the host when the layer goes).
    held: Arc<AtomicBool>,
    /// The host's own object for this buffer (its `wl_buffer`), made once, dropped with the slot.
    host: std::sync::Mutex<Option<Box<dyn std::any::Any + Send>>>,
}

impl Slot {
    /// A `size` slot for frames copied as `format`, with one of the host's `offered`
    /// (fourcc, modifier) pairs, on `device`.
    pub(crate) fn new(
        device: &wgpu::Device,
        size: PhysicalSize<u32>,
        format: wgpu::TextureFormat,
        offered: &[(u32, u64)],
    ) -> Result<Self, ImportError> {
        let &(_, drm_format, vk_format) = SLOT_FORMATS
            .iter()
            .find(|row| row.0 == format)
            .ok_or_else(|| ImportError::VulkanImport(format!("no OS buffer format for {format:?}")))?;
        let vk_err =
            |what: &'static str| move |e: vk::Result| ImportError::VulkanImport(format!("{what}: {e}"));
        // SAFETY: the raw handles stay inside wgpu's device; the image is handed to wgpu with a
        // drop callback that frees it, and the exported fd is owned by the slot.
        unsafe {
            let hal = device
                .as_hal::<wgpu::hal::api::Vulkan>()
                .ok_or(ImportError::VulkanImport("not a Vulkan device".into()))?;
            let raw = hal.raw_device().clone();
            let instance = hal.shared_instance().raw_instance().clone();
            let physical = hal.raw_physical_device();
            // the driver's modifiers for the format: counted, then read
            let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
            instance.get_physical_device_format_properties2(
                physical,
                vk_format,
                &mut vk::FormatProperties2::default().push_next(&mut list),
            );
            let mut props =
                vec![vk::DrmFormatModifierPropertiesEXT::default(); list.drm_format_modifier_count as usize];
            let mut list =
                vk::DrmFormatModifierPropertiesListEXT::default().drm_format_modifier_properties(&mut props);
            instance.get_physical_device_format_properties2(
                physical,
                vk_format,
                &mut vk::FormatProperties2::default().push_next(&mut list),
            );
            let driver: Vec<(u64, u32, vk::FormatFeatureFlags)> = props
                .iter()
                .map(|p| {
                    (
                        p.drm_format_modifier,
                        p.drm_format_modifier_plane_count,
                        p.drm_format_modifier_tiling_features,
                    )
                })
                .collect();
            let modifiers = slot_modifiers(&driver, offered, drm_format);
            let mut list_info =
                vk::ImageDrmFormatModifierListCreateInfoEXT::default().drm_format_modifiers(&modifiers);
            let mut external = vk::ExternalMemoryImageCreateInfo::default()
                .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            let image = raw
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(vk_format)
                        .extent(vk::Extent3D {
                            width: size.width,
                            height: size.height,
                            depth: 1,
                        })
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                        .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
                        .sharing_mode(vk::SharingMode::EXCLUSIVE)
                        .initial_layout(vk::ImageLayout::UNDEFINED)
                        .push_next(&mut external)
                        .push_next(&mut list_info),
                    None,
                )
                .map_err(vk_err("vkCreateImage"))?;
            let req = raw.get_image_memory_requirements(image);
            let types = instance.get_physical_device_memory_properties(physical);
            let Some(index) = (0..types.memory_type_count).find(|&i| {
                req.memory_type_bits & (1 << i) != 0
                    && types.memory_types[i as usize]
                        .property_flags
                        .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            }) else {
                raw.destroy_image(image, None);
                return Err(ImportError::VulkanImport("no device-local memory for an OS buffer".into()));
            };
            let mut export = vk::ExportMemoryAllocateInfo::default()
                .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
            let memory = match raw.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(index)
                    .push_next(&mut export)
                    .push_next(&mut dedicated),
                None,
            ) {
                Ok(m) => m,
                Err(e) => {
                    raw.destroy_image(image, None);
                    return Err(vk_err("vkAllocateMemory")(e));
                }
            };
            let free = |raw: &ash::Device| {
                raw.destroy_image(image, None);
                raw.free_memory(memory, None);
            };
            if let Err(e) = raw.bind_image_memory(image, memory, 0) {
                free(&raw);
                return Err(vk_err("vkBindImageMemory")(e));
            }
            let fd = match ash::khr::external_memory_fd::Device::new(&instance, &raw).get_memory_fd(
                &vk::MemoryGetFdInfoKHR::default()
                    .memory(memory)
                    .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT),
            ) {
                Ok(fd) => OwnedFd::from_raw_fd(fd),
                Err(e) => {
                    free(&raw);
                    return Err(vk_err("vkGetMemoryFdKHR")(e));
                }
            };
            let mut chosen = vk::ImageDrmFormatModifierPropertiesEXT::default();
            if let Err(e) = ash::ext::image_drm_format_modifier::Device::new(&instance, &raw)
                .get_image_drm_format_modifier_properties(image, &mut chosen)
            {
                free(&raw);
                return Err(vk_err("vkGetImageDrmFormatModifierPropertiesEXT")(e));
            }
            let layout = raw.get_image_subresource_layout(
                image,
                vk::ImageSubresource {
                    aspect_mask: vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
                    mip_level: 0,
                    array_layer: 0,
                },
            );
            let extent = wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            };
            let drop_device = raw.clone();
            let hal_texture = hal.texture_from_raw(
                image,
                &wgpu::hal::TextureDescriptor {
                    label: Some("weld-os-buffer"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUses::COPY_DST | wgpu::TextureUses::RESOURCE,
                    memory_flags: wgpu::hal::MemoryFlags::empty(),
                    view_formats: Vec::new(),
                },
                Some(Box::new(move || {
                    drop_device.destroy_image(image, None);
                    drop_device.free_memory(memory, None);
                })),
                wgpu::hal::vulkan::TextureMemory::External,
            );
            drop(hal);
            let texture = device.create_texture_from_hal::<wgpu::hal::api::Vulkan>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("weld-os-buffer"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::TextureUses::UNINITIALIZED,
            );
            Ok(Self {
                texture,
                fd,
                offset: layout.offset as u32,
                stride: layout.row_pitch as u32,
                modifier: chosen.drm_format_modifier,
                drm_format,
                size,
                held: Arc::new(AtomicBool::new(false)),
                host: std::sync::Mutex::new(None),
            })
        }
    }

    /// The frame copy's destination.
    pub(crate) fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// The compositor holds it (attached and not yet released).
    pub(crate) fn compositor_uses(&self) -> bool {
        self.held.load(Ordering::Acquire)
    }
}

/// wubble change (71): what a host needs to show a Linux OS buffer in the compositor.
impl OsBuffer {
    /// The dmabuf of its one plane, valid while this buffer is alive.
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.0.fd.as_fd()
    }

    /// The plane's byte offset and row stride.
    pub fn offset_stride(&self) -> (u32, u32) {
        (self.0.offset, self.0.stride)
    }

    /// The DRM fourcc and modifier it was made with.
    pub fn drm_format(&self) -> (u32, u64) {
        (self.0.drm_format, self.0.modifier)
    }

    pub fn size(&self) -> PhysicalSize<u32> {
        self.0.size
    }

    /// Whether the compositor holds it: the host sets it as it hands the buffer over and clears
    /// it when the compositor lets it go; the pool copies into no held slot. Shared, so the
    /// host's release handler may own a clone.
    pub fn compositor_hold(&self) -> &Arc<AtomicBool> {
        &self.0.held
    }

    /// The host's own object for this buffer (made once by the host, dropped with the slot).
    pub fn host_data(&self) -> &std::sync::Mutex<Option<Box<dyn std::any::Any + Send>>> {
        &self.0.host
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XR24: u32 = fourcc(b"XR24");
    const XB24: u32 = fourcc(b"XB24");
    const BLOCK: u64 = 0x0300_0000_0060_6014;
    const OTHER: u64 = 0x0300_0000_0060_6015;

    #[test]
    fn a_slot_takes_the_modifiers_both_sides_have() {
        let copy_and_sample = vk::FormatFeatureFlags::TRANSFER_DST | vk::FormatFeatureFlags::SAMPLED_IMAGE;
        let copy_only = vk::FormatFeatureFlags::TRANSFER_DST;
        type Row = (&'static str, &'static [(u64, u32, vk::FormatFeatureFlags)], &'static [(u32, u64)], Vec<u64>);
        let rows: Vec<Row> = vec![
            ("the common one", &[(BLOCK, 1, copy_and_sample), (OTHER, 1, copy_and_sample)], &[(XR24, BLOCK)], vec![BLOCK]),
            ("both common, both kept", &[(BLOCK, 1, copy_and_sample), (LINEAR, 1, copy_and_sample)], &[(XR24, LINEAR), (XR24, BLOCK)], vec![BLOCK, LINEAR]),
            ("offered for another fourcc only: LINEAR", &[(BLOCK, 1, copy_and_sample)], &[(XB24, BLOCK)], vec![LINEAR]),
            ("not samplable: LINEAR", &[(BLOCK, 1, copy_only)], &[(XR24, BLOCK)], vec![LINEAR]),
            ("two planes: LINEAR", &[(BLOCK, 2, copy_and_sample)], &[(XR24, BLOCK)], vec![LINEAR]),
            ("the compositor said nothing: LINEAR", &[(BLOCK, 1, copy_and_sample)], &[], vec![LINEAR]),
            ("the driver has none: LINEAR", &[], &[(XR24, BLOCK)], vec![LINEAR]),
        ];
        for (name, driver, offered, want) in rows {
            assert_eq!(slot_modifiers(driver, offered, XR24), want, "{name}");
        }
    }

    #[test]
    fn every_frame_copy_format_has_a_slot_format() {
        for format in [wgpu::TextureFormat::Bgra8UnormSrgb, wgpu::TextureFormat::Rgba8UnormSrgb] {
            assert!(SLOT_FORMATS.iter().any(|r| r.0 == format), "{format:?}");
        }
    }
}
