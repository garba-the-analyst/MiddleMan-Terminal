use crate::state::AppState;
use rand::RngCore;
use uuid::Uuid;

pub async fn ensure_wallets(state: &AppState, user_id: Uuid) -> anyhow::Result<()> {
    // Ensure NGN wallet exists
    let _ = mm_db::queries::ensure_ngn_wallet(&state.pool, user_id).await;

    // Provision crypto wallets if vault is available and row missing
    if let Some(vault) = &state.vault {
        for chain in ["SOLANA", "EVM"] {
            if mm_db::queries::get_key_vault(&state.pool, user_id, chain)
                .await
                .unwrap_or(None)
                .is_some()
            {
                continue;
            }
            let (address, encrypted) = generate_and_encrypt(vault, user_id, chain)?;
            let _ = mm_db::queries::insert_key_vault(&state.pool, user_id, chain, &address, &encrypted).await;
        }
    } else {
        // Vault disabled — create mock rows without encryption for demo
        for chain in ["SOLANA", "EVM"] {
            if mm_db::queries::get_key_vault(&state.pool, user_id, chain)
                .await
                .unwrap_or(None)
                .is_some()
            {
                continue;
            }
            let (address, _) = generate_mock_address(chain);
            let _ = mm_db::queries::insert_key_vault(
                &state.pool,
                user_id,
                chain,
                &address,
                "vault-disabled-mock",
            )
            .await;
        }
    }
    Ok(())
}

fn generate_and_encrypt(
    vault: &mm_vault::VaultAead,
    user_id: Uuid,
    chain: &str,
) -> anyhow::Result<(String, String)> {
    let (address, priv_hex) = generate_mock_address(chain);
    let aad = format!("mm:vault:v1:{user_id}:{chain}");
    let encrypted = vault.encrypt(priv_hex.as_bytes(), &aad).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok((address, encrypted))
}

fn generate_mock_address(chain: &str) -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let priv_hex = hex::encode(bytes);
    let address = match chain {
        "SOLANA" => bs58::encode(&bytes).into_string(),
        "EVM" => {
            // Mock EVM address: 0x + last 20 bytes hex
            let mut addr_bytes = [0u8; 20];
            addr_bytes.copy_from_slice(&bytes[12..32]);
            format!("0x{}", hex::encode(addr_bytes))
        }
        _ => bs58::encode(&bytes).into_string(),
    };
    (address, priv_hex)
}

pub async fn wallet_summary(state: &AppState, user_id: Uuid) -> String {
    let wallets = mm_db::queries::list_wallets(&state.pool, user_id)
        .await
        .unwrap_or_default();
    let keys = {
        let mut out = Vec::new();
        for chain in ["SOLANA", "EVM"] {
            if let Ok(Some(k)) = mm_db::queries::get_key_vault(&state.pool, user_id, chain).await {
                out.push(format!("{}: {}", chain, &k.public_address[..12.min(k.public_address.len())]));
            }
        }
        out
    };

    if wallets.is_empty() && keys.is_empty() {
        return "No wallets yet — send any message to provision.".into();
    }

    let mut lines = vec!["💼 Your wallets:".to_string()];
    for w in wallets {
        lines.push(format!("{}: {} {}", w.currency, w.balance, w.wallet_type));
    }
    for k in keys {
        lines.push(k);
    }
    lines.join("\n")
}
