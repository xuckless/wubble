// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Permission requests: a page asks for the camera, the user's location, or
//! anything else Chromium gates behind a prompt.
//!
//! CEF asks through two different callbacks — an ordinary prompt, and a
//! separate one for camera and microphone capture — and each takes a different
//! answer: the prompt wants an accept/deny result, media wants the requested
//! capture bits handed back (or 0). Both are reference-counted and may
//! be answered later, so a host is given one id and one pair of verbs and this
//! module remembers which kind it was.
//!
//! Answering is opt-in for the same reason as auth: an unanswered request holds
//! the page waiting forever. With `handle_permission_requests` off, a request
//! is reported and immediately denied, which is what a browser does when the
//! user dismisses a prompt.

use std::sync::Mutex;

/// Identifies one permission request until it is answered.
pub type PermissionId = crate::held::RequestId;

/// What a page asked for.
///
/// Chromium gates far more than this behind prompts; these are the ones worth
/// naming. Anything else arrives as [`PermissionKind::Other`] carrying CEF's
/// raw bit, so a host can still report it rather than see nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PermissionKind {
    CameraStream,
    MicStream,
    Geolocation,
    Notifications,
    Clipboard,
    MidiSysex,
    PointerLock,
    KeyboardLock,
    IdleDetection,
    LocalFonts,
    StorageAccess,
    ProtectedMediaIdentifier,
    DesktopAudioCapture,
    DesktopVideoCapture,
    /// A bit this build does not name, kept so nothing is silently dropped.
    Other(u32),
}

/// CEF's `cef_permission_request_types_t`, as of CEF 147. Read off
/// `include/internal/cef_types.h` in the distribution rather than guessed;
/// anything not named here still reaches the host as `Other`.
const CAMERA_STREAM: u32 = 1 << 2;
const CLIPBOARD: u32 = 1 << 4;
const LOCAL_FONTS: u32 = 1 << 7;
const GEOLOCATION: u32 = 1 << 8;
const IDLE_DETECTION: u32 = 1 << 11;
const MIC_STREAM: u32 = 1 << 12;
const MIDI_SYSEX: u32 = 1 << 13;
const NOTIFICATIONS: u32 = 1 << 15;
const KEYBOARD_LOCK: u32 = 1 << 16;
const POINTER_LOCK: u32 = 1 << 17;
const PROTECTED_MEDIA_IDENTIFIER: u32 = 1 << 18;
const STORAGE_ACCESS: u32 = 1 << 20;

/// Split CEF's bitmask into named permissions.
pub(crate) fn decode(mask: u32) -> Vec<PermissionKind> {
    let mut out = Vec::new();
    let mut seen = 0u32;
    for (bit, kind) in [
        (CAMERA_STREAM, PermissionKind::CameraStream),
        (MIC_STREAM, PermissionKind::MicStream),
        (GEOLOCATION, PermissionKind::Geolocation),
        (NOTIFICATIONS, PermissionKind::Notifications),
        (CLIPBOARD, PermissionKind::Clipboard),
        (MIDI_SYSEX, PermissionKind::MidiSysex),
        (POINTER_LOCK, PermissionKind::PointerLock),
        (KEYBOARD_LOCK, PermissionKind::KeyboardLock),
        (LOCAL_FONTS, PermissionKind::LocalFonts),
        (IDLE_DETECTION, PermissionKind::IdleDetection),
        (STORAGE_ACCESS, PermissionKind::StorageAccess),
        (
            PROTECTED_MEDIA_IDENTIFIER,
            PermissionKind::ProtectedMediaIdentifier,
        ),
    ] {
        if mask & bit != 0 {
            out.push(kind);
            seen |= bit;
        }
    }
    // Whatever is left is real but unnamed here; report it rather than drop it.
    let mut rest = mask & !seen;
    while rest != 0 {
        let bit = rest & rest.wrapping_neg();
        out.push(PermissionKind::Other(bit));
        rest &= !bit;
    }
    out
}

/// CEF's `cef_media_access_permission_types_t`. A different enum from the
/// prompt bits above, on a different callback -- `1 << 0` means audio capture
/// here and nothing at all there, so they must not share a decoder.
const MEDIA_DEVICE_AUDIO: u32 = 1 << 0;
const MEDIA_DEVICE_VIDEO: u32 = 1 << 1;
const MEDIA_DESKTOP_AUDIO: u32 = 1 << 2;
const MEDIA_DESKTOP_VIDEO: u32 = 1 << 3;

/// Split CEF's media-capture bitmask into named permissions.
pub(crate) fn decode_media(mask: u32) -> Vec<PermissionKind> {
    let mut out = Vec::new();
    let mut seen = 0u32;
    for (bit, kind) in [
        (MEDIA_DEVICE_AUDIO, PermissionKind::MicStream),
        (MEDIA_DEVICE_VIDEO, PermissionKind::CameraStream),
        (MEDIA_DESKTOP_AUDIO, PermissionKind::DesktopAudioCapture),
        (MEDIA_DESKTOP_VIDEO, PermissionKind::DesktopVideoCapture),
    ] {
        if mask & bit != 0 {
            out.push(kind);
            seen |= bit;
        }
    }
    let mut rest = mask & !seen;
    while rest != 0 {
        let bit = rest & rest.wrapping_neg();
        out.push(PermissionKind::Other(bit));
        rest &= !bit;
    }
    out
}

/// Which CEF callback a pending request came from; they answer differently.
pub(crate) enum Pending {
    /// wubble change: CEF's `prompt_id` is kept so `OnDismissPermissionPrompt`
    /// can find the request.
    #[cfg(feature = "cef-runtime")]
    Prompt {
        callback: cef::PermissionPromptCallback,
        requested: u32,
        prompt_id: u64,
    },
    /// The mask is kept because granting media means handing the requested
    /// bits back, not a boolean.
    #[cfg(feature = "cef-runtime")]
    Media(cef::MediaAccessCallback, u32),
    #[cfg(not(feature = "cef-runtime"))]
    Unwired,
}

/// wubble change: whether a request for `asked` is granted when the host
/// grants `granted`. Both CEF callbacks take one answer for the whole set:
/// the prompt one accept/deny, and the media one the requested bits back
/// exactly (`CefMediaAccessCallback::Continue`, cef_permission_handler.h:
/// for getUserMedia "|allowed_permissions| must match
/// |required_permissions|"). So a request is granted only when every kind it
/// asked for was granted.
pub(crate) fn all_granted(asked: &[PermissionKind], granted: &[PermissionKind]) -> bool {
    !asked.is_empty() && asked.iter().all(|kind| granted.contains(kind))
}

// wubble change: requests are held in a `crate::held::Held` (id, cap, default
// answer on close and drop) instead of an id counter and a map.
#[derive(Default)]
pub(crate) struct Permissions {
    enabled: Mutex<bool>,
    #[cfg(feature = "cef-runtime")]
    pub(crate) pending: Mutex<crate::held::Held<Pending>>,
}

impl Permissions {
    pub(crate) fn set_enabled(&self, enabled: bool) {
        *self.enabled.lock().unwrap() = enabled;
    }

    pub(crate) fn is_enabled(&self) -> bool {
        *self.enabled.lock().unwrap()
    }

    // Diagnostic accessor: exercised by unit tests today, kept for host-side
    // leak checks.
    #[allow(dead_code)]
    pub(crate) fn outstanding(&self) -> usize {
        #[cfg(feature = "cef-runtime")]
        {
            return self.pending.lock().unwrap().outstanding();
        }
        #[cfg(not(feature = "cef-runtime"))]
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bits_seen_from_a_real_page_decode() {
        // Observed on https://example.com: getCurrentPosition produced 0x100,
        // getUserMedia({audio:true}) produced 0x1 on the media callback.
        assert_eq!(decode(0x100), vec![PermissionKind::Geolocation]);
        assert_eq!(decode(1 << 15), vec![PermissionKind::Notifications]);
        assert_eq!(decode(1 << 2), vec![PermissionKind::CameraStream]);
    }

    #[test]
    fn several_at_once_all_come_back() {
        let got = decode(GEOLOCATION | NOTIFICATIONS | CAMERA_STREAM);
        assert_eq!(got.len(), 3);
        assert!(got.contains(&PermissionKind::Geolocation));
        assert!(got.contains(&PermissionKind::Notifications));
        assert!(got.contains(&PermissionKind::CameraStream));
    }

    #[test]
    fn an_unnamed_bit_is_reported_not_dropped() {
        let odd = 1 << 30;
        assert_eq!(decode(odd), vec![PermissionKind::Other(odd)]);
        // and alongside a known one
        let got = decode(GEOLOCATION | odd);
        assert_eq!(
            got.len(),
            2,
            "a known bit swallowed the unknown one: {got:?}"
        );
        assert!(got.contains(&PermissionKind::Other(odd)));
    }

    #[test]
    fn every_named_bit_is_distinct() {
        // A copy-paste in the constant table would make two names share a bit
        // and silently mis-report what a page asked for.
        let named = [
            CAMERA_STREAM,
            MIC_STREAM,
            GEOLOCATION,
            NOTIFICATIONS,
            CLIPBOARD,
            MIDI_SYSEX,
            POINTER_LOCK,
            KEYBOARD_LOCK,
            LOCAL_FONTS,
            IDLE_DETECTION,
            STORAGE_ACCESS,
            PROTECTED_MEDIA_IDENTIFIER,
        ];
        let mut seen = named.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), named.len(), "two permissions share a bit");
        for bit in named {
            assert_eq!(decode(bit).len(), 1, "{bit:#x} decoded to more than one");
        }
    }

    #[test]
    fn nothing_requested_decodes_to_nothing() {
        assert!(decode(0).is_empty());
    }

    #[test]
    fn answering_is_off_until_the_host_asks_for_it() {
        let p = Permissions::default();
        assert!(
            !p.is_enabled(),
            "an unwired host must not leave pages waiting on a prompt"
        );
        p.set_enabled(true);
        assert!(p.is_enabled());
    }

    #[test]
    fn nothing_is_outstanding_before_a_request() {
        assert_eq!(Permissions::default().outstanding(), 0);
    }

    #[test]
    fn a_request_is_granted_only_when_everything_asked_is_granted() {
        let asked = decode(GEOLOCATION | NOTIFICATIONS);
        let both = [PermissionKind::Geolocation, PermissionKind::Notifications];
        assert!(all_granted(&asked, &both));
        assert!(!all_granted(&asked, &both[..1]), "a partial grant accepted the whole prompt");
        assert!(!all_granted(&asked, &[]));
        assert!(!all_granted(&[], &both), "nothing asked is nothing to accept");
    }

    #[test]
    fn camera_and_microphone_are_granted_together_or_not_at_all() {
        // CEF denies a getUserMedia answer that is a subset of the request.
        let asked = decode_media(MEDIA_DEVICE_AUDIO | MEDIA_DEVICE_VIDEO);
        assert!(!all_granted(&asked, &[PermissionKind::CameraStream]));
        assert!(!all_granted(&asked, &[PermissionKind::MicStream]));
        assert!(all_granted(
            &asked,
            &[PermissionKind::CameraStream, PermissionKind::MicStream]
        ));
        assert!(!all_granted(&decode_media(MEDIA_DEVICE_AUDIO), &[PermissionKind::DesktopVideoCapture]));
    }
}
