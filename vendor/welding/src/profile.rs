// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Per-producer CEF request contexts.
//!
//! A CEF `Settings::root_cache_path` belongs to the whole process. Passing no
//! request context when a browser is created therefore quietly puts every
//! producer in the global context, even when its public surface configuration
//! claims a separate user-data directory. Creating a context here is the
//! ownership boundary: one surface, one cookie/storage/cache profile.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cef::rc::Rc;
use cef::{ImplRequestContextHandler, RequestContextHandler, WrapRequestContextHandler};

use crate::{WeldError, runtime::CefRuntime, surface::CefSurfaceConfig};

// CEF invokes this on its UI thread only after a RequestContext can create
// browsers. A persistent child profile initializes asynchronously.
// wubble change 46: moved here from `macos_cef` so every platform's shared
// profile can wait for it.
cef::wrap_request_context_handler! {
    pub(crate) struct RequestContextReadyHandler { ready: Arc<AtomicBool> }

    impl RequestContextHandler {
        fn on_request_context_initialized(&self, _request_context: Option<&mut cef::RequestContext>) {
            self.ready.store(true, Ordering::Release);
        }
    }
}

/// One CEF request context that several producers share: one cookie jar,
/// local storage, HTTP cache, permission store and service-worker set.
/// Hand a clone to each producer through
/// [`CefSurfaceConfig::profile`]; a producer without one still gets its own
/// context (`create` below). Drop every clone before `cef::shutdown`.
///
/// CEF initializes a disk-backed context asynchronously; create browsers in it
/// only once [`Self::is_ready`] (pump CEF until then).
/// wubble change 46, see `WUBBLE_CHANGES.md`.
#[derive(Clone)]
pub struct SharedProfile {
    context: cef::RequestContext,
    ready: Arc<AtomicBool>,
}

impl std::fmt::Debug for SharedProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedProfile")
            .field("ready", &self.is_ready())
            .finish_non_exhaustive()
    }
}

impl SharedProfile {
    /// A context on `user_data_dir` (absolute, inside the runtime's
    /// `cache_path`), session cookies persisted; `None` is in memory.
    pub fn new(runtime: &CefRuntime, user_data_dir: Option<PathBuf>) -> Result<Self, WeldError> {
        let config = CefSurfaceConfig {
            user_data_dir,
            ..CefSurfaceConfig::default()
        };
        let ready = Arc::new(AtomicBool::new(false));
        let mut handler = RequestContextReadyHandler::new(ready.clone());
        let context = create_with_handler(runtime, &config, Some(&mut handler))?;
        Ok(Self { context, ready })
    }

    /// CEF reported the context initialized: browsers can be created in it.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// The CEF context, for its cookie manager, cache and preferences.
    pub fn request_context(&self) -> cef::RequestContext {
        self.context.clone()
    }
}

/// The context a producer creates its browser in: the shared one when the
/// config names it, else a fresh one of its own.
pub(crate) fn for_surface(
    runtime: &CefRuntime,
    config: &CefSurfaceConfig,
) -> Result<cef::RequestContext, WeldError> {
    match &config.profile {
        Some(profile) => Ok(profile.request_context()),
        None => create(runtime, config),
    }
}

/// Build the distinct request context owned by one producer.
///
/// An empty CEF cache path is intentionally still passed through a fresh
/// context. It produces an isolated in-memory profile, rather than selecting
/// CEF's process-global context.
pub(crate) fn create(
    runtime: &CefRuntime,
    config: &CefSurfaceConfig,
) -> Result<cef::RequestContext, WeldError> {
    create_with_handler(runtime, config, None)
}

/// Build one context and optionally receive CEF's asynchronous initialization
/// callback. macOS uses the callback to defer browser creation until its host
/// event loop has finished bringing a persistent child profile online.
pub(crate) fn create_with_handler(
    runtime: &CefRuntime,
    config: &CefSurfaceConfig,
    handler: Option<&mut cef::RequestContextHandler>,
) -> Result<cef::RequestContext, WeldError> {
    let mut settings = cef::RequestContextSettings::default();

    if let Some(path) = config.user_data_dir.as_ref() {
        if !path.is_absolute() {
            return Err(WeldError::SurfaceCreation(format!(
                "CefSurfaceConfig::user_data_dir must be absolute: {}",
                path.display()
            )));
        }

        let root = runtime.config().cache_path.as_ref().ok_or_else(|| {
            WeldError::SurfaceCreation(
                "CefRuntimeConfig::cache_path is required when a producer uses user_data_dir; \
                 otherwise CEF selects a shared platform-default root cache"
                    .into(),
            )
        })?;
        if !root.is_absolute() {
            return Err(WeldError::SurfaceCreation(format!(
                "CefRuntimeConfig::cache_path must be absolute for persistent profiles: {}",
                root.display()
            )));
        }
        // wubble change 46: CEF's Chrome runtime makes a disk-backed context a
        // Chrome profile, which must be a direct child of the root
        // (`chrome_browser_context.cc` logs "Cannot create profile at path"
        // and falls back to an in-memory profile without telling the caller).
        if path.parent() != Some(root.as_path()) {
            return Err(WeldError::SurfaceCreation(format!(
                "CefSurfaceConfig::user_data_dir ({}) must be a direct child of CefRuntimeConfig::cache_path ({})",
                path.display(),
                root.display(),
            )));
        }

        let path: cef::CefString = path.to_string_lossy().as_ref().into();
        settings.cache_path = path;
        // Persistent contexts should retain session cookies as well as the
        // normal persistent cookie jar. An in-memory context has no disk to
        // retain them on, so leaving the CEF default there is correct.
        settings.persist_session_cookies = 1;
    }

    cef::request_context_create_context(Some(&settings), handler).ok_or_else(|| {
        WeldError::SurfaceCreation("cef_request_context_create_context returned None".into())
    })
}
