mod c_add_resource_pack;
mod c_config_disconnect;
mod c_cookie_request;
mod c_finish_config;
mod c_known_packs;
mod c_plugin_message;
mod c_registry_data;

pub use c_add_resource_pack::*;
pub use c_config_disconnect::*;
pub use c_cookie_request::*;
pub use c_finish_config::*;
pub use c_known_packs::*;
pub use c_plugin_message::*;
pub use c_registry_data::*;

/// DO NOT CHANGE ORDER
/// This Enum has the exact order like vanilla, Vanilla parses their Packet IDs from the enum order. Its also way easier to port.
#[repr(i32)]
pub enum ClientboundConfigPackets {
    CookieRequest,
    PluginMessage,
    Disconnect,
    Finish,
    KeepAlive,
    Ping,
    ResetChat,
    RegistryData,
    RemoveResourcePack,
    AddResourcePack,
    StoreCookie,
    Transfer,
    FeatureFlags,
    UpdateTags,
    KnownPacks,
    CustomReportDetails,
    ServerLinks,
}
