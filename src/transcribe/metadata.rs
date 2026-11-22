//! Measurements reported by the engine instance that handled a transcription.
//! Unknown values stay absent; binary capabilities and configured device hints
//! are not runtime evidence.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RuntimeMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_load_ms: Option<u64>,
}
