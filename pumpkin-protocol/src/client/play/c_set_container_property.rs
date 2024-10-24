use pumpkin_macros::client_packet;
use serde::Serialize;

use crate::VarInt;

use super::ClientboundPlayPackets;
#[derive(Serialize)]
#[client_packet(ClientboundPlayPackets::WindowProperty as i32)]
pub struct CSetContainerProperty {
    window_id: VarInt,
    property: i16,
    value: i16,
}

impl CSetContainerProperty {
    pub const fn new(window_id: VarInt, property: i16, value: i16) -> Self {
        Self {
            window_id,
            property,
            value,
        }
    }
}
