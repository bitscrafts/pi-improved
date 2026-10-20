//! Spec 214: an unknown kind byte is a refusal that names the byte, never a default kind.
//! Bytes are taken from the top of the range so that a future variant appended at 4 does not
//! change this test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_memory::doc::{DecodeError, decode_kind};

fn payload(first: u8) -> Vec<u8> {
    let mut b = vec![first];
    b.extend_from_slice(&7u64.to_le_bytes());
    b.extend_from_slice(b"some text");
    b
}

#[test]
fn unknown_kind_byte_refuses_naming_the_byte() {
    for byte in [0x80u8, 0xff] {
        let err = decode_kind(Some(payload(byte))).expect_err("an unknown kind must refuse");
        assert_eq!(err, DecodeError::UnknownKind { byte });
        assert!(
            err.to_string().contains(&format!("0x{byte:02x}")),
            "refusal must name the byte: {err}"
        );
    }
}

#[test]
fn truncated_payload_refuses_instead_of_defaulting() {
    let err = decode_kind(Some(vec![1, 2, 3])).expect_err("a short payload must refuse");
    assert_eq!(err, DecodeError::Truncated { len: 3 });
}
