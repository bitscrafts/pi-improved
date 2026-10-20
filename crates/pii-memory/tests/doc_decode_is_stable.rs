//! Spec 214: a stored doc decodes to the same kind and text across a build change, and the
//! byte layout and serde names of `DocKind` stay fixed. The fixture holds 40 docs sampled from
//! the production `memory.redb` (2026-10-20), with the kind and text the pre-change decoder gave.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_memory::doc::{Doc, DocKind, Layer, decode_kind, decode_text, encode};

const FIXTURE: &str = include_str!("fixtures/prod_docs_sample.json");

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn production_sample_decodes_to_the_same_kinds_and_text() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(FIXTURE).unwrap();
    assert_eq!(rows.len(), 40);
    for row in &rows {
        let id = row["id"].as_str().unwrap();
        let payload = unhex(row["payload_hex"].as_str().unwrap());
        let want_kind = row["kind"].as_str().unwrap();
        let want_text = row["text"].as_str().unwrap();
        assert_eq!(
            format!("{:?}", decode_kind(Some(payload.clone())).unwrap()),
            want_kind,
            "kind of {id}"
        );
        assert_eq!(decode_text(Some(payload)), want_text, "text of {id}");
    }
}

#[test]
fn kind_bytes_and_serde_names_are_fixed() {
    for (kind, byte, name) in [
        (DocKind::Atom, 0u8, "atom"),
        (DocKind::Scene, 1, "scene"),
        (DocKind::Persona, 2, "persona"),
        (DocKind::Skill, 3, "skill"),
    ] {
        let doc = Doc::new(Layer::L1, kind, "body", vec![], 1.0, 9);
        let bytes = encode(&doc);
        assert_eq!(bytes.first().copied(), Some(byte), "byte of {kind:?}");
        assert_eq!(
            bytes.get(1..9),
            Some(&9u64.to_le_bytes()[..]),
            "created_ms layout"
        );
        assert_eq!(decode_text(Some(bytes.clone())), "body");
        assert_eq!(decode_kind(Some(bytes)).unwrap(), kind);
        assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{name}\""));
    }
}
