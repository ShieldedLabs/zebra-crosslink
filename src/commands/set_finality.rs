#![allow(deprecated)]
use std::{num::NonZeroUsize, str::FromStr};

use anyhow::anyhow;
use clap::Args;
use secrecy::ExposeSecret;
use uuid::Uuid;

use zcash_address::ZcashAddress;
use zcash_client_backend::{
    data_api::{
        wallet::{
            create_proposed_transactions, input_selection::GreedyInputSelector, propose_transfer,
        },
        Account, WalletRead,
    },
    fees::{standard::MultiOutputChangeStrategy, DustOutputPolicy, SplitPolicy, StandardFeeRule},
    proto::service,
    wallet::OvkPolicy,
};
use zcash_client_sqlite::{util::SystemClock, WalletDb};
use zcash_keys::keys::UnifiedSpendingKey;
use zcash_proofs::prover::LocalTxProver;
use zcash_protocol::{
    memo::{Memo, MemoBytes},
    value::Zatoshis,
    ShieldedProtocol,
};
use zip321::{Payment, TransactionRequest};

use crate::{
    commands::select_account,
    config::WalletConfig,
    data::get_db_paths,
    error,
    remote::{tor_client, Servers},
    MIN_CONFIRMATIONS,
};

// Options accepted for the `send` command
#[derive(Debug, Args)]
pub(crate) struct Command {
    /// The server to send via (default is \"ecc\")
    #[arg(short, long)]
    #[arg(default_value = "ecc", value_parser = Servers::parse)]
    server: Servers,

    block_hash: String,
}

impl Command {
    pub(crate) async fn run(self) -> Result<(), anyhow::Error> {
        let server = self.server.pick(zcash_protocol::consensus::Network::MainNetwork)?;

        let mut hash = hex::decode(self.block_hash).unwrap();
        hash.reverse();
        let hash = zcash_primitives::block::BlockHash(hash.try_into().unwrap());

        let mut client = server.connect_direct().await?;
        let response = client.send_fiat_finality(zcash_client_backend::proto::service::BlockId { hash: hash.0.into(), height: 0 }).await?.into_inner();

        if response.error_code != 0 {
            Err(error::Error::SendFailed {
                code: response.error_code,
                reason: response.error_message,
            }
            .into())
        } else {
            println!("Command was successful.");
            Ok(())
        }
    }
}
