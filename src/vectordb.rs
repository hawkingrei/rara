use anyhow::Result;

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct MemoryMetadata {
    pub session_id: String,
    pub turn_index: u32,
    pub text: String,
}

pub struct VectorDB {
    _uri: String,
}

impl VectorDB {
    pub fn new(uri: &str) -> Self {
        Self {
            _uri: uri.to_string(),
        }
    }

    pub async fn search_with_metadata(
        &self,
        _table_name: &str,
        _query_vector: Vec<f32>,
        _limit: usize,
    ) -> Result<Vec<(MemoryMetadata, f32)>> {
        // Mocked for compilation
        Ok(vec![])
    }

    pub async fn upsert_turn(
        &self,
        _table_name: &str,
        _metadata: MemoryMetadata,
        _vector: Vec<f32>,
    ) -> Result<()> {
        // Mocked for compilation
        Ok(())
    }
}
