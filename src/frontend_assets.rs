#[derive(Debug, Clone, Copy)]
pub struct EmbeddedAsset {
    pub path: &'static str,
    pub bytes: &'static [u8],
    pub mime: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/frontend_assets.rs"));

pub fn embedded_frontend_assets() -> &'static [EmbeddedAsset] {
    FRONTEND_ASSETS
}
