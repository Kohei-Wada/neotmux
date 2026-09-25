//! Decode then encode: a key a terminal sends, read by the client and written into a pane by the
//! server, comes back as the bytes it started as. The one test that needs both halves, so it lives
//! in the crate that has both.

use neotmux::client::decode::Decoder;
use neotmux::server::input::encode_key;
use neotmux_proto::keys::Key;

fn one(bytes: &[u8]) -> Key {
    let keys = Decoder::default().feed(bytes);
    assert_eq!(
        keys.len(),
        1,
        "expected one key from {bytes:?}, got {keys:?}"
    );
    keys[0]
}

/// The `u`-form is a real key, alongside every other form it can arrive as — the
/// round-trip holds: decode then encode comes back to the bytes a terminal would send
/// for that key.
#[test]
fn a_csi_u_key_round_trips_to_its_conventional_bytes() {
    assert_eq!(
        encode_key(one(b"\x1b[27u"), false).as_deref(),
        Some(&[0x1b][..])
    );
    assert_eq!(
        encode_key(one(b"\x1b[97u"), false).as_deref(),
        Some(b"a".as_slice())
    );
}

/// Every key form the decoder produces has to survive the trip back to the bytes a
/// terminal would have sent, because that is the trip a keystroke makes into a pane.
#[test]
fn decoding_bytes_and_encoding_them_again_is_the_identity() {
    for bytes in [
        &b"a"[..],
        "あ".as_bytes(),
        &[0x02],
        b"\r",
        b"\t",
        &[0x7f],
        &[0x1b],
        b"\x1ba",
        b"\x1b[A",
        b"\x1b[B",
        b"\x1b[C",
        b"\x1b[D",
        b"\x1b[H",
        b"\x1b[F",
        b"\x1b[Z",
        b"\x1b[2~",
        b"\x1b[3~",
        b"\x1b[5~",
        b"\x1b[6~",
        b"\x1b[15~",
        b"\x1b[24~",
        b"\x1bOP",
        b"\x1bOS",
        b"\x1b[1;5D",
        b"\x1b[1;3A",
        b"\x1b[1;2A",
        b"\x1b[1;8A",
        b"\x1b[5;5~",
        b"\x1b[24;3~",
        b"\x1b[1;5P",
    ] {
        let keys = Decoder::default().feed(bytes);
        assert_eq!(keys.len(), 1, "{bytes:?} did not decode to one key");
        assert_eq!(
            encode_key(keys[0], false).as_deref(),
            Some(bytes),
            "{bytes:?} did not survive the round trip as {:?}",
            keys[0]
        );
    }
}
