use alloy::{
    network::EthereumWallet,
    primitives::{Address, U256},
    providers::ProviderBuilder,
    signers::local::PrivateKeySigner,
    sol,
};
use eyre::Result;
use tracing::{error, info};

use crate::{config::Config, scanner::Opportunity};

/// Huntloan contract interface (matches Huntloan.sol)
sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface IHuntloan {
        function requestFlashLiquidation(
            address debtAsset,
            uint256 debtAmount,
            address collateralAsset,
            address borrower
        ) external;
    }
}

/// Execute a flash loan liquidation via Huntloan.sol
pub async fn execute(cfg: &Config, opp: &Opportunity) -> Result<()> {
    // Build a wallet-backed provider
    let signer: PrivateKeySigner = cfg.operator_key.parse()?;
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc_http.parse()?);

    let contract = IHuntloan::new(cfg.huntloan_addr, provider.clone());

    let debt_amount = U256::from(opp.debt_to_repay);

    info!(
        borrower = %opp.borrower,
        debt_amount = %debt_amount,
        estimated_profit = opp.estimated_profit_usd,
        "Firing flash loan liquidation"
    );

    // Simulate first (eth_call) — if this reverts we pay no gas
    let call = contract.requestFlashLiquidation(
        opp.debt_asset,
        debt_amount,
        opp.collateral_asset,
        opp.borrower,
    );

    match call.call().await {
        Err(e) => {
            error!("Simulation reverted — aborting: {e}");
            return Ok(()); // silent abort, no gas wasted
        }
        Ok(_) => info!("Simulation passed — broadcasting tx"),
    }

    // Broadcast
    let receipt = call.send().await?.get_receipt().await?;
    info!(
        tx_hash = %receipt.transaction_hash,
        block  = receipt.block_number.unwrap_or(0),
        "TX confirmed"
    );

    Ok(())
}
