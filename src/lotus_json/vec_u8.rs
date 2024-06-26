// Copyright 2019-2024 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use super::*;

// This code looks odd so we can
// - use #[serde(with = "...")]
// - de/ser empty vecs as null
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "Base64String")]
pub struct VecU8LotusJson(Option<Inner>);

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
struct Inner(
    #[serde(with = "base64_standard")]
    #[schemars(with = "String")]
    Vec<u8>,
);

impl HasLotusJson for Vec<u8> {
    type LotusJson = VecU8LotusJson;

    #[cfg(test)]
    fn snapshots() -> Vec<(serde_json::Value, Self)> {
        vec![
            (json!("aGVsbG8gd29ybGQh"), Vec::from_iter(*b"hello world!")),
            (json!(null), Vec::new()),
        ]
    }

    fn into_lotus_json(self) -> Self::LotusJson {
        match self.is_empty() {
            true => VecU8LotusJson(None),
            false => VecU8LotusJson(Some(Inner(self))),
        }
    }

    fn from_lotus_json(value: Self::LotusJson) -> Self {
        match value {
            VecU8LotusJson(Some(Inner(vec))) => vec,
            VecU8LotusJson(None) => Vec::new(),
        }
    }
}
