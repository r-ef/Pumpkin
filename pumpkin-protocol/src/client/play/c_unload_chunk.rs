use pumpkin_macros::client_packet;
use serde::Serialize;

use super::ClientboundPlayPackets;

#[derive(Serialize)]
#[client_packet(ClientboundPlayPackets::UnloadChunk as i32)]
pub struct CUnloadChunk {
    z: i32,
    x: i32,
}

impl CUnloadChunk {
    pub fn new(x: i32, z: i32) -> Self {
        Self { z, x }
    }
}
