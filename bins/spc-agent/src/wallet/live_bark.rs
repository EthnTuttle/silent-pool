//! Stock wallet opening on the AP-3a persister. No seed regeneration, second
//! funds ledger, server extension or assumed server fee schedule.

use anyhow::{ensure, Context};
use ark::ProtocolEncoding;
use bitcoin::secp256k1::PublicKey;

use super::{BarkStore, WalletKeys};

pub struct LiveBark {
    pub(crate) wallet: bark::Wallet,
    pub(crate) endpoint: String,
    pub(crate) server_pubkey: PublicKey,
    pub(crate) mailbox_pubkey: PublicKey,
    pub(super) ui_observations: super::ui_inputs::Observations,
}

impl LiveBark {
    pub async fn open(
        keys: &WalletKeys,
        store: &BarkStore,
        config: &crate::config::Ark,
        expected_id: [u8; 4],
    ) -> anyhow::Result<Self> {
        let server_pubkey: PublicKey = config
            .server_pubkey
            .as_deref()
            .context("ark.server_pubkey required")?
            .parse()?;
        let mailbox_pubkey: PublicKey = config
            .mailbox_pubkey
            .as_deref()
            .context("ark.mailbox_pubkey required")?
            .parse()?;
        ensure!(
            <[u8; 4]>::from(ark::address::ArkId::from_server_pubkey(server_pubkey)) == expected_id,
            "wrong-ark-server"
        );
        let endpoint = config
            .server_address
            .clone()
            .context("ark.server_address required")?;
        let network = super::keys::bitcoin_network(keys.network);
        // Validate actual metadata before stock refresh_server can persist first-use keys.
        let server = server_rpc::ServerConnection::builder()
            .address(&endpoint)
            .network(network)
            .connect()
            .await
            .context("ark connection unavailable")?;
        let info = server.ark_info().await;
        ensure!(
            info.server_pubkey == server_pubkey && info.mailbox_pubkey == mailbox_pubkey,
            "wrong-ark-server"
        );
        info.fees.validate().context("invalid Ark fee schedule")?;
        let props = store
            .properties()
            .await?
            .context("missing bark properties")?;
        ensure!(
            props.server_pubkey.is_none_or(|p| p == server_pubkey)
                && props
                    .server_mailbox_pubkey
                    .is_none_or(|p| p == mailbox_pubkey),
            "stored Ark server identity mismatch"
        );
        let mut stock = bark::Config::network_default(network);
        stock.server_address = endpoint.clone();
        stock.esplora_address = None;
        stock.bitcoind_address = Some(
            config
                .chain_rpc_url
                .clone()
                .context("ark.chain_rpc_url required")?,
        );
        stock.bitcoind_cookiefile = Some(
            config
                .chain_cookie
                .clone()
                .context("ark.chain_cookie required")?,
        );
        let wallet = bark::Wallet::open(
            network,
            keys.bark_seed(),
            stock,
            bark::OpenWalletArgs {
                datadir: Some(
                    store
                        .db_path()
                        .parent()
                        .context("bark directory")?
                        .to_path_buf(),
                ),
                persister: Some(store.persister()),
                create_if_not_exists: false,
                run_daemon: false,
                ..Default::default()
            },
        )
        .await
        .context("stock Wallet::open")?;
        wallet.refresh_server().await?;
        ensure!(
            wallet.mailbox_identifier().serialize() == keys.mailbox_public().serialize(),
            "stock mailbox differs from provisioned child seed"
        );
        Ok(Self {
            wallet,
            endpoint,
            server_pubkey,
            mailbox_pubkey,
            ui_observations: Default::default(),
        })
    }

    /// Liveness and policy validation only. Opening an unfunded wallet does not
    /// establish a received balance or a synchronized SPc scan cursor.
    pub async fn check_connection(&self) -> anyhow::Result<()> {
        self.wallet.refresh_server().await?;
        let info = self.wallet.require_ark_info().await?;
        ensure!(
            info.server_pubkey == self.server_pubkey && info.mailbox_pubkey == self.mailbox_pubkey,
            "wrong-ark-server"
        );
        info.fees.validate()?;
        Ok(())
    }
}
