use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoIdentity {
    pub id: String,
    pub display_name: String,
    pub unique_name: String,
}
