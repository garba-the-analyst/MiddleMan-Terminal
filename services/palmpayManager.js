import pool from '../config/db.js';
import axios from 'axios';

// Palmpay API credentials from your .env
const PALMPAY_MERCHANT_ID = process.env.PALMPAY_MERCHANT_ID;
const PALMPAY_API_KEY = process.env.PALMPAY_API_KEY;
const PALMPAY_BASE_URL = "https://api.palmpay.com/v1"; 

export async function sendNairaToBank(userId, bankCode, accountNumber, amount) {
    const client = await pool.connect();
    
    try {
        // 1. Fetch the user's current Naira balance from the Neon Fiat Wallet
        const ledgerResult = await client.query(
            "SELECT balance FROM fiat_wallets WHERE user_id = $1 AND currency = 'NGN'",
            [userId]
        );

        if (ledgerResult.rows.length === 0) {
            return { status: "FAILED", message: "Could not locate your Naira wallet." };
        }

        const currentBalance = parseFloat(ledgerResult.rows[0].balance);

        if (currentBalance < amount) {
            return { status: "FAILED", message: `Insufficient funds. Your balance is ₦${currentBalance}.` };
        }

        // 2. Call Palmpay API (Mocked setup for now until you add real API keys)
        const transactionReference = `MM-T-${Date.now()}`; 
        
        // Note: In production, uncomment the axios call below.
        /*
        const palmpayResponse = await axios.post(`${PALMPAY_BASE_URL}/transfer`, {
            merchantId: PALMPAY_MERCHANT_ID,
            reference: transactionReference,
            amount: amount,
            currency: "NGN",
            destination: { type: "bank_account", accountNumber: accountNumber, bankCode: bankCode },
            reason: "MiddleMan Withdrawal"
        }, { headers: { 'Authorization': `Bearer ${PALMPAY_API_KEY}` } });
        */
        
        // Simulating a successful API response for testing
        const isSuccess = true; 

        // 3. Verify the transfer was successful and update the database
        if (isSuccess) {
            await client.query('BEGIN'); // Start transaction

            const newBalance = currentBalance - amount;
            
            // Deduct from wallet
            await client.query(
                "UPDATE fiat_wallets SET balance = $1 WHERE user_id = $2 AND currency = 'NGN'",
                [newBalance, userId]
            );

            // Log the transaction
            await client.query(
                "INSERT INTO transactions (user_id, type, amount, currency, reference, status) VALUES ($1, $2, $3, $4, $5, 'COMPLETED')",
                [userId, 'PALMPAY_WITHDRAWAL', amount, 'NGN', transactionReference]
            );

            await client.query('COMMIT');
            return { status: "SUCCESS", message: `Successfully sent ₦${amount} to ${accountNumber}. New balance is ₦${newBalance}.` };
        } else {
            return { status: "FAILED", message: "Bank transfer failed at the gateway." };
        }

    } catch (error) {
        await client.query('ROLLBACK');
        console.error("Palmpay Transfer Error:", error);
        return { status: "ERROR", message: "We encountered an issue processing your transfer." };
    } finally {
        client.release();
    }
}