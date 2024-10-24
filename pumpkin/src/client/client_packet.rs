use num_traits::FromPrimitive;
use pumpkin_config::{ADVANCED_CONFIG, BASIC_CONFIG};
use pumpkin_core::text::TextComponent;
use pumpkin_protocol::{
    client::{
        config::{CConfigAddResourcePack, CFinishConfig, CKnownPacks, CRegistryData},
        login::{CLoginSuccess, CSetCompression},
        status::CPingResponse,
    },
    server::{
        config::{SAcknowledgeFinishConfig, SClientInformationConfig, SKnownPacks, SPluginMessage},
        handshake::SHandShake,
        login::{SEncryptionResponse, SLoginAcknowledged, SLoginPluginResponse, SLoginStart},
        status::{SStatusPingRequest, SStatusRequest},
    },
    ConnectionState, KnownPack, CURRENT_MC_PROTOCOL,
};
use uuid::Uuid;

use crate::{
    client::authentication::{self, offline_uuid, validate_textures, GameProfile},
    entity::player::{ChatMode, Hand},
    proxy::{
        bungeecord,
        velocity::{self, velocity_login},
    },
    server::{Server, CURRENT_MC_VERSION},
};

use super::{authentication::AuthError, Client, PlayerConfig};

/// Processes incoming Packets from the Client to the Server
/// Implements the `Client` Packets
/// NEVER TRUST THE CLIENT. HANDLE EVERY ERROR, UNWRAP/EXPECT
impl Client {
    pub async fn handle_handshake(&self, handshake: SHandShake) {
        let version = handshake.protocol_version.0;
        self.protocol_version
            .store(version, std::sync::atomic::Ordering::Relaxed);
        *self.server_address.lock().await = handshake.server_address;

        log::debug!("Handshake: next state {:?}", &handshake.next_state);
        self.connection_state.store(handshake.next_state);
        if self.connection_state.load() != ConnectionState::Status {
            let protocol = version;
            match protocol.cmp(&(CURRENT_MC_PROTOCOL as i32)) {
                std::cmp::Ordering::Less => {
                    self.kick(&format!("Client outdated ({protocol}), Server uses Minecraft {CURRENT_MC_VERSION}, Protocol {CURRENT_MC_PROTOCOL}")).await;
                }
                std::cmp::Ordering::Equal => {}
                std::cmp::Ordering::Greater => {
                    self.kick(&format!("Server outdated, Server uses Minecraft {CURRENT_MC_VERSION}, Protocol {CURRENT_MC_PROTOCOL}")).await;
                }
            }
        }
    }

    pub async fn handle_status_request(&self, server: &Server, _status_request: SStatusRequest) {
        log::debug!("Handling status request for id");
        let status = server.get_status();
        self.send_packet(&status.lock().await.get_status()).await;
    }

    pub async fn handle_ping_request(&self, ping_request: SStatusPingRequest) {
        log::debug!("Handling ping request for id");
        self.send_packet(&CPingResponse::new(ping_request.payload))
            .await;
        self.close();
    }

    fn is_valid_player_name(name: &str) -> bool {
        name.len() <= 16
            && name
                .chars()
                .all(|c| c > 32_u8 as char && c < 127_u8 as char)
    }

    pub async fn handle_login_start(&self, server: &Server, login_start: SLoginStart) {
        log::debug!("login start");

        if !Self::is_valid_player_name(&login_start.name) {
            self.kick("Invalid characters in username").await;
            return;
        }
        // default game profile, when no online mode
        // TODO: make offline uuid
        let mut gameprofile = self.gameprofile.lock().await;
        let proxy = &ADVANCED_CONFIG.proxy;
        if proxy.enabled {
            if proxy.velocity.enabled {
                velocity_login(self).await;
            } else if proxy.bungeecord.enabled {
                match bungeecord::bungeecord_login(self, login_start.name).await {
                    Ok((_ip, profile)) => {
                        // self.address.lock() = ip;
                        self.finish_login(&profile).await;
                        *gameprofile = Some(profile);
                    }
                    Err(error) => self.kick(&error.to_string()).await,
                }
            }
        } else {
            let id = if BASIC_CONFIG.online_mode {
                login_start.uuid
            } else {
                offline_uuid(&login_start.name).expect("This is very not safe and bad")
            };

            let profile = GameProfile {
                id,
                name: login_start.name,
                properties: vec![],
                profile_actions: None,
            };

            if BASIC_CONFIG.encryption {
                let verify_token: [u8; 4] = rand::random();
                self.send_packet(
                    &server.encryption_request(&verify_token, BASIC_CONFIG.online_mode),
                )
                .await;
            } else {
                if ADVANCED_CONFIG.packet_compression.enabled {
                    self.enable_compression().await;
                }
                self.finish_login(&profile).await;
            }

            *gameprofile = Some(profile);
        }
    }

    pub async fn handle_encryption_response(
        &self,
        server: &Server,
        encryption_response: SEncryptionResponse,
    ) {
        log::debug!("Handling encryption for id");
        let shared_secret = server.decrypt(&encryption_response.shared_secret).unwrap();

        if let Err(error) = self.set_encryption(Some(&shared_secret)).await {
            self.kick(&error.to_string()).await;
            return;
        }

        let mut gameprofile = self.gameprofile.lock().await;

        let Some(profile) = gameprofile.as_mut() else {
            self.kick("No Game profile").await;
            return;
        };

        if BASIC_CONFIG.online_mode {
            match self
                .autenticate(server, &shared_secret, &profile.name)
                .await
            {
                Ok(new_profile) => *profile = new_profile,
                Err(e) => {
                    self.kick(&e.to_string()).await;
                    return;
                }
            }
        }

        if ADVANCED_CONFIG.packet_compression.enabled {
            self.enable_compression().await;
        }
        self.finish_login(profile).await;
    }

    async fn enable_compression(&self) {
        let compression = ADVANCED_CONFIG.packet_compression.compression_info.clone();
        self.send_packet(&CSetCompression::new(compression.threshold.into()))
            .await;
        self.set_compression(Some(compression)).await;
    }

    async fn finish_login(&self, profile: &GameProfile) {
        let packet = CLoginSuccess::new(&profile.id, &profile.name, &profile.properties);
        self.send_packet(&packet).await;
    }

    async fn autenticate(
        &self,
        server: &Server,
        shared_secret: &[u8],
        username: &str,
    ) -> Result<GameProfile, AuthError> {
        if let Some(auth_client) = &server.auth_client {
            let hash = server.digest_secret(shared_secret);
            let ip = self.address.lock().await.ip();

            let profile = authentication::authenticate(username, &hash, &ip, auth_client).await?;
            // Check if player should join
            if let Some(actions) = &profile.profile_actions {
                if ADVANCED_CONFIG
                    .authentication
                    .player_profile
                    .allow_banned_players
                {
                    for allowed in &ADVANCED_CONFIG
                        .authentication
                        .player_profile
                        .allowed_actions
                    {
                        if !actions.contains(allowed) {
                            return Err(AuthError::DisallowedAction);
                        }
                    }
                    if !actions.is_empty() {
                        return Err(AuthError::Banned);
                    }
                } else if !actions.is_empty() {
                    return Err(AuthError::Banned);
                }
            }
            // validate textures
            for property in &profile.properties {
                validate_textures(property, &ADVANCED_CONFIG.authentication.textures)
                    .map_err(AuthError::TextureError)?;
            }
            return Ok(profile);
        }
        Err(AuthError::MissingAuthClient)
    }

    pub async fn handle_plugin_response(&self, plugin_response: SLoginPluginResponse) {
        log::debug!("Handling plugin for id");
        let velocity_config = &ADVANCED_CONFIG.proxy.velocity;
        if velocity_config.enabled {
            let mut address = self.address.lock().await;
            match velocity::receive_velocity_plugin_response(
                address.port(),
                velocity_config,
                plugin_response,
            ) {
                Ok((profile, new_address)) => {
                    self.finish_login(&profile).await;
                    *self.gameprofile.lock().await = Some(profile);
                    *address = new_address;
                }
                Err(error) => self.kick(&error.to_string()).await,
            }
        }
    }

    pub async fn handle_login_acknowledged(
        &self,
        server: &Server,
        _login_acknowledged: SLoginAcknowledged,
    ) {
        log::debug!("Handling login acknowledged for id");
        self.connection_state.store(ConnectionState::Config);
        self.send_packet(&server.get_branding()).await;

        let resource_config = &ADVANCED_CONFIG.resource_pack;
        if resource_config.enabled {
            let resource_pack = CConfigAddResourcePack::new(
                Uuid::new_v3(
                    &uuid::Uuid::NAMESPACE_DNS,
                    resource_config.resource_pack_url.as_bytes(),
                ),
                &resource_config.resource_pack_url,
                &resource_config.resource_pack_sha1,
                resource_config.force,
                if resource_config.prompt_message.is_empty() {
                    None
                } else {
                    Some(TextComponent::text(&resource_config.prompt_message))
                },
            );

            self.send_packet(&resource_pack).await;
        }

        // known data packs
        self.send_packet(&CKnownPacks::new(&[KnownPack {
            namespace: "minecraft",
            id: "core",
            version: "1.21",
        }]))
        .await;
        log::debug!("login acknowledged");
    }
    pub async fn handle_client_information_config(
        &self,
        client_information: SClientInformationConfig,
    ) {
        log::debug!("Handling client settings for id");
        if let (Some(main_hand), Some(chat_mode)) = (
            Hand::from_i32(client_information.main_hand.into()),
            ChatMode::from_i32(client_information.chat_mode.into()),
        ) {
            *self.config.lock().await = Some(PlayerConfig {
                locale: client_information.locale,
                view_distance: client_information.view_distance,
                chat_mode,
                chat_colors: client_information.chat_colors,
                skin_parts: client_information.skin_parts,
                main_hand,
                text_filtering: client_information.text_filtering,
                server_listing: client_information.server_listing,
            });
        } else {
            self.kick("Invalid hand or chat type").await;
        }
    }

    pub async fn handle_plugin_message(&self, plugin_message: SPluginMessage) {
        log::debug!("Handling plugin message for id");
        if plugin_message.channel.starts_with("minecraft:brand")
            || plugin_message.channel.starts_with("MC|Brand")
        {
            log::debug!("got a client brand");
            match String::from_utf8(plugin_message.data) {
                Ok(brand) => *self.brand.lock().await = Some(brand),
                Err(e) => self.kick(&e.to_string()).await,
            }
        }
    }

    pub async fn handle_known_packs(&self, server: &Server, _config_acknowledged: SKnownPacks) {
        log::debug!("Handling known packs for id");
        for registry in &server.cached_registry {
            self.send_packet(&CRegistryData::new(
                &registry.registry_id,
                &registry.registry_entries,
            ))
            .await;
        }

        // We are done with configuring
        log::debug!("finished config");
        self.send_packet(&CFinishConfig::new()).await;
    }

    pub fn handle_config_acknowledged(&self, _config_acknowledged: &SAcknowledgeFinishConfig) {
        log::debug!("Handling config acknowledge for id");
        self.connection_state.store(ConnectionState::Play);
        self.make_player
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
