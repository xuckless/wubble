//! The vault lifecycle as one enum and one pure transition function.

use zeroize::Zeroizing;

/// The master key, zeroed on drop.
pub type Key = Zeroizing<[u8; 32]>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultState {
    Locked,
    Unlocked {
        key: Key,
    },
    /// No usable keyring (or a corrupt key entry). Nothing is saved or decrypted.
    Unavailable {
        reason: String,
    },
    /// The keyring has no key, but `rows` saved logins were sealed with one: no new key is
    /// made over them (it would replace the lost one for good). Nothing is saved or decrypted
    /// until the key is back (a retry) or the owner deletes them and starts over.
    KeyLost {
        rows: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    UnlockRequested,
    KeyLoaded(Key),
    /// The keyring has no key; `stored` logins are in the store.
    KeyMissing {
        stored: usize,
    },
    KeyringFailed(String),
    LockRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Read the key from the keyring.
    LoadKey,
    /// Create a random key in the keyring (after `KeyMissing`).
    CreateKey,
}

/// Pure transition. Unavailable only leaves on an explicit `UnlockRequested`
/// (retry is a user action, never automatic).
pub fn transition(state: VaultState, event: Event) -> (VaultState, Vec<Effect>) {
    use VaultState::*;
    match (state, event) {
        (Unlocked { key }, Event::UnlockRequested) => (Unlocked { key }, vec![]),
        (_, Event::UnlockRequested) => (Locked, vec![Effect::LoadKey]),
        (Locked, Event::KeyLoaded(key)) => (Unlocked { key }, vec![]),
        (Locked, Event::KeyMissing { stored: 0 }) => (Locked, vec![Effect::CreateKey]),
        (Locked, Event::KeyMissing { stored }) => (KeyLost { rows: stored }, vec![]),
        (Locked, Event::KeyringFailed(reason)) => (Unavailable { reason }, vec![]),
        (Unavailable { reason }, Event::LockRequested) => (Unavailable { reason }, vec![]),
        (KeyLost { rows }, Event::LockRequested) => (KeyLost { rows }, vec![]),
        (_, Event::LockRequested) => (Locked, vec![]),
        // Late results in any other state are ignored.
        (state, Event::KeyLoaded(_) | Event::KeyMissing { .. } | Event::KeyringFailed(_)) => {
            (state, vec![])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> Key {
        Zeroizing::new([7; 32])
    }
    fn unavailable() -> VaultState {
        VaultState::Unavailable { reason: "x".into() }
    }
    fn unlocked() -> VaultState {
        VaultState::Unlocked { key: key() }
    }

    #[test]
    fn table() {
        use Event::*;
        use VaultState::Locked;
        let cases: Vec<(VaultState, Event, VaultState, Vec<Effect>)> = vec![
            (Locked, UnlockRequested, Locked, vec![Effect::LoadKey]),
            (Locked, KeyLoaded(key()), unlocked(), vec![]),
            // a key is made only for an empty store; logins sealed with another key wait for it
            (
                Locked,
                KeyMissing { stored: 0 },
                Locked,
                vec![Effect::CreateKey],
            ),
            (
                Locked,
                KeyMissing { stored: 2 },
                VaultState::KeyLost { rows: 2 },
                vec![],
            ),
            (
                VaultState::KeyLost { rows: 2 },
                UnlockRequested,
                Locked,
                vec![Effect::LoadKey],
            ),
            (
                VaultState::KeyLost { rows: 2 },
                LockRequested,
                VaultState::KeyLost { rows: 2 },
                vec![],
            ),
            (
                Locked,
                KeyringFailed("e".into()),
                VaultState::Unavailable { reason: "e".into() },
                vec![],
            ),
            (Locked, LockRequested, Locked, vec![]),
            (unlocked(), UnlockRequested, unlocked(), vec![]),
            (
                unlocked(),
                KeyLoaded(Zeroizing::new([1; 32])),
                unlocked(),
                vec![],
            ),
            (unlocked(), KeyMissing { stored: 0 }, unlocked(), vec![]),
            (unlocked(), KeyringFailed("e".into()), unlocked(), vec![]),
            (unlocked(), LockRequested, Locked, vec![]),
            (
                unavailable(),
                UnlockRequested,
                Locked,
                vec![Effect::LoadKey],
            ),
            (unavailable(), KeyLoaded(key()), unavailable(), vec![]),
            (
                unavailable(),
                KeyMissing { stored: 0 },
                unavailable(),
                vec![],
            ),
            (
                unavailable(),
                KeyringFailed("e".into()),
                unavailable(),
                vec![],
            ),
            (unavailable(), LockRequested, unavailable(), vec![]),
        ];
        for (state, event, want_state, want_effects) in cases {
            let label = format!("{state:?} + {event:?}");
            let (s, e) = transition(state, event);
            assert_eq!((s, e), (want_state, want_effects), "{label}");
        }
    }
}
