import { ethers } from "ethers";
import pool from '../config/db.js';
import crypto from "crypto";

// A 32-byte secret key stored in your .env file for encryption
const ENCRYPTION_KEY = process.env.ENCRYPTION_KEY; 

// Utility to encrypt private keys
function encrypt(text) {
    const iv = crypto.randomBytes(16);
    const cipher = crypto.createCipheriv('aes-256-cbc', Buffer.from(ENCRYPTION_KEY), iv);
    let encrypted = cipher.update(text);
    encrypted = Buffer.concat([encrypted, cipher.final()]);
    return iv.toString('hex') + ':' + encrypted.toString('hex');
}

/**
 * Onboards a new MiddleMan user directly into the Neon PostgreSQL database
 * @param {string} whatsappNumber - The user's phone number
 */
export async function onboardNewUser(whatsappNumber) {
    console.log(`Checking ledger for: ${whatsappNumber}`);
    
    // We grab a dedicated client from the pool to run a clean transaction
    const client = await pool.connect();

    try {
        // Start SQL transaction
        await client.query('BEGIN');

        // 1. Check if user exists
        const userCheck = await client.query(
            'SELECT id FROM users WHERE whatsapp_number = $1',
            [whatsappNumber]
        );

        if (userCheck.rows.length > 0) {
            await client.query('ROLLBACK');
            return { status: "EXISTING_USER", message: "User already has an account." };
        }

        // 2. Create the User
        const insertUser = await client.query(
            'INSERT INTO users (whatsapp_number) VALUES ($1) RETURNING id',
            [whatsappNumber]
        );
        const newUserId = insertUser.rows[0].id;

        // 3. Initialize Naira Balance
        await client.query(
            'INSERT INTO balances (user_id, currency, balance) VALUES ($1, $2, $3)',
            [newUserId, 'NGN', 0.00]
        );

        // 4. Generate Web3 EVM Wallet
        const evmWallet = ethers.Wallet.createRandom();
        const encryptedEvmKey = encrypt(evmWallet.privateKey);

        // 5. Save Wallet to Ledger
        await client.query(
            'INSERT INTO web3_wallets (user_id, chain_family, public_address, encrypted_private_key) VALUES ($1, $2, $3, $4)',
            [newUserId, 'EVM', evmWallet.address, encryptedEvmKey]
        );

        // Commit transaction if everything succeeded
        await client.query('COMMIT');
        
        console.log(`Onboarding complete! EVM Address: ${evmWallet.address}`);
        
        return { 
            status: "SUCCESS", 
            userId: newUserId,
            evmAddress: evmWallet.address 
        };

    } catch (error) {
        await client.query('ROLLBACK');
        console.error("Error during onboarding:", error);
        throw error;
    } finally {
        // Always release the client back to the pool
        client.release();
    }
}