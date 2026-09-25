//! The contract between the two spellings of a key.
//!
//! `neotmux_server::input::key_spec` turns the `Key` a terminal reports into a spec string, and
//! `neotmux_lua::api::keymap::normalise` turns the spec string a plugin types into the form
//! that is stored and looked up. They are written in two crates that share no code, and the
//! only thing in the whole program that makes them agree is that both sides end up under the
//! same `normalise`. That is easy to break and impossible to see: `dispatch_key` runs
//! `normalise` on what `key_spec` produced, and when `normalise` refuses it the key falls
//! through with `KeyOutcome::Fallthrough` — no log, no error, and every other test still
//! passes, because none of them puts a `key_spec` output through `normalise`.
//!
//! This file is the only place the two are checked against each other. It pins:
//! 1. no key the host can report has a spec that the plugin side refuses (the silent-drop); and
//! 2. the name a plugin author would type for a key is the very key the host reports for it —
//!    including its modifiers, in any order.

use neotmux_lua::api::keymap::normalise;
use neotmux_proto::keys::{Key, KeyCode, Modifiers};
use neotmux_server::input::key_spec;

/// What a key comes to once both sides have had their say at the shared `normalise`: the
/// host's report of the key, and — separately — the plugin's spelling of it.
fn canonical(key: &Key) -> Result<String, String> {
    normalise(&key_spec(key))
}

fn bound(spec: &str) -> String {
    normalise(spec).expect("a binding spelling under test must itself be valid")
}

/// Every code the host can report: characters (plain, shifted-looking, control, non-ASCII),
/// every named key, and the whole F1–F12 run.
fn all_codes() -> Vec<KeyCode> {
    [
        KeyCode::Char('a'),
        KeyCode::Char('Z'),
        KeyCode::Char(' '),
        KeyCode::Char('あ'),
        KeyCode::Enter,
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Backspace,
        KeyCode::Esc,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Insert,
        KeyCode::Delete,
    ]
    .into_iter()
    .chain((1u8..=12).map(KeyCode::F))
    .collect()
}

/// Every modifier bit a terminal can report — `from_param` admits 1..=8.
fn all_mods() -> Vec<Modifiers> {
    (0u8..8)
        .map(|b| Modifiers {
            shift: b & 1 != 0,
            alt: b & 2 != 0,
            ctrl: b & 4 != 0,
        })
        .collect()
}

/// If any of these is refused, a plugin binding that key is a binding that can never fire.
#[test]
fn no_key_the_host_reports_is_silently_refused() {
    for code in all_codes() {
        for mods in all_mods() {
            let key = Key::new(code, mods);
            let spec = key_spec(&key);
            assert!(
                normalise(&spec).is_ok(),
                "the plugin side refused the spec the host produces for {key:?}: '{spec}'"
            );
        }
    }
}

/// The spelling a plugin author would write for a key must come to the same canonical form as
/// the key the host reports for that key. The names deliberately use the human's case — the
/// only reason `enter` and the host's `Enter` agree today is that both are re-normalised.
#[test]
fn the_name_a_plugin_types_for_a_key_is_the_key_the_host_reports() {
    let mut pairs: Vec<(String, KeyCode)> = vec![
        ("enter".into(), KeyCode::Enter),
        ("tab".into(), KeyCode::Tab),
        // shift-tab, whose intended binding name the host itself calls `BackTab`
        // (see `neotmux-client`'s `decoder.rs`, where `ESC [ Z` becomes a plain `BackTab`): the point of this
        // test is that a person who writes `backtab` binds the very key it reports.
        ("backtab".into(), KeyCode::BackTab),
        ("backspace".into(), KeyCode::Backspace),
        ("esc".into(), KeyCode::Esc),
        ("up".into(), KeyCode::Up),
        ("down".into(), KeyCode::Down),
        ("left".into(), KeyCode::Left),
        ("right".into(), KeyCode::Right),
        ("home".into(), KeyCode::Home),
        ("end".into(), KeyCode::End),
        ("pageup".into(), KeyCode::PageUp),
        ("pagedown".into(), KeyCode::PageDown),
        ("insert".into(), KeyCode::Insert),
        ("delete".into(), KeyCode::Delete),
    ];
    for n in 1u8..=12 {
        pairs.push((format!("f{n}"), KeyCode::F(n)));
    }

    for (human, code) in &pairs {
        let event = Key::plain(*code);
        assert_eq!(
            bound(human),
            canonical(&event).expect("the host's report of this key must be bindable"),
            "binding '{human}' does not come to the key the host reports for it"
        );
    }
}

/// Modifiers must attach to the key, and the plugin may write them in any order. If the two
/// sides disagreed on a `C-S-Up` versus the host's `C-Up`-shifted form, the modifier keys
/// would fall through while the plain ones worked — exactly the half-broken state that is
/// hardest to notice.
#[test]
fn modifiers_stay_attached_to_the_named_binding_in_any_order() {
    let cases = [
        ("c-enter", Key::new(KeyCode::Enter, Modifiers::CTRL)),
        ("m-left", Key::new(KeyCode::Left, Modifiers::ALT)),
        (
            "s-c-up",
            Key::new(
                KeyCode::Up,
                Modifiers {
                    shift: true,
                    ctrl: true,
                    ..Modifiers::NONE
                },
            ),
        ),
        (
            "m-c-delete",
            Key::new(
                KeyCode::Delete,
                Modifiers {
                    alt: true,
                    ctrl: true,
                    ..Modifiers::NONE
                },
            ),
        ),
    ];
    for (spec, key) in cases {
        assert_eq!(
            bound(spec),
            canonical(&key).expect("the host's report of this key must be bindable"),
            "'{spec}' must come to the same key the host reports"
        );
    }
}
