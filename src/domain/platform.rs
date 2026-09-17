use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Platform {
    pub id: i64,
    pub name: String,
    pub display_name: String,
    pub slug: String,
    pub fs_slug: String,
    pub rom_count: i64,
}
