//! Closed lab-only pre-sign construction effect. This is not an arbitrary
//! transaction-version option. Absent effects preserve normal version-2 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "schema", deny_unknown_fields)]
pub enum ConstructionEffect {
    #[serde(rename = "spc-lab-version-rejection/v1")]
    VersionRejectionV1,
}
impl<'de> serde::Deserialize<'de> for ConstructionEffect {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Internally tagged unit variants discard remaining fields. A strict
        // struct keeps this tag closed; serialized bytes remain identical.
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema: String,
        }
        let wire = <Wire as serde::Deserialize>::deserialize(deserializer)?;
        if wire.schema == "spc-lab-version-rejection/v1" {
            Ok(Self::VersionRejectionV1)
        } else {
            Err(serde::de::Error::custom(
                "unsupported construction effect schema",
            ))
        }
    }
}
impl ConstructionEffect {
    pub(crate) fn version(effect: Option<Self>) -> bitcoin::transaction::Version {
        match effect {
            Some(Self::VersionRejectionV1) => bitcoin::transaction::Version(3),
            None => bitcoin::transaction::Version::TWO,
        }
    }
}
