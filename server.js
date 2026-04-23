import express from 'express';
import cors from 'cors';
import 'dotenv/config';
import pool from './config/db.js';
import { whatsappClient } from './services/whatsapp.js';

const app = express();
const PORT = process.env.PORT || 3000;

// --- MIDDLEWARE ---
// Allows your Vue.js dashboard to securely communicate with this API
app.use(cors());
// Allows the server to parse incoming JSON payloads
app.use(express.json()); 


// --- ADMIN DASHBOARD API ROUTES ---

// 1. Get all pending gift card trades
app.get('/api/admin/giftcards/pending', async (req, res) => {
    try {
        const result = await pool.query(`
            SELECT g.*, u.full_name, u.whatsapp_number 
            FROM gift_card_trades g 
            JOIN users u ON g.user_id = u.id 
            WHERE g.status = 'pending'
            ORDER BY g.created_at DESC
        `);
        res.json(result.rows);
    } catch (err) {
        console.error("Error fetching pending trades:", err);
        res.status(500).json({ error: "Database error" });
    }
});

// 2. Approve a gift card trade
app.post('/api/admin/giftcards/approve', async (req, res) => {
    const { tradeId, adminPin } = req.body;

    // Hardcoded Admin Security for now
    if (adminPin !== "9999") return res.status(403).json({ error: "Unauthorized" });

    const client = await pool.connect();
    try {
        await client.query('BEGIN');

        // Fetch trade details
        const tradeRes = await client.query("SELECT * FROM gift_card_trades WHERE id = $1", [tradeId]);
        if (tradeRes.rows.length === 0) throw new Error("Trade not found");
        const trade = tradeRes.rows[0];

        // Update trade status
        await client.query("UPDATE gift_card_trades SET status = 'approved' WHERE id = $1", [tradeId]);

        // Credit User's FIAT (Naira) Wallet
        await client.query(`
            INSERT INTO wallets (user_id, wallet_type, currency, balance) 
            VALUES ($1, 'FIAT', 'NGN', $2) 
            ON CONFLICT (user_id, currency, wallet_type) 
            DO UPDATE SET balance = wallets.balance + EXCLUDED.balance
        `, [trade.user_id, trade.offered_naira_value]);

        // Fetch user phone for notification
        const userRes = await client.query("SELECT whatsapp_number FROM users WHERE id = $1", [trade.user_id]);
        const userPhone = userRes.rows[0].whatsapp_number;

        await client.query('COMMIT');

        // Trigger WhatsApp Notification immediately after approval
        whatsappClient.sendMessage(userPhone, `✅ *Gift Card Approved!*\n\nYour trade for the $${trade.amount_in_usd} ${trade.card_category} has been approved. ₦${parseFloat(trade.offered_naira_value).toLocaleString()} has been added to your FIAT wallet.\n\nType /bal to check.`);

        res.json({ success: true, message: "Trade approved and user credited." });

    } catch (err) {
        await client.query('ROLLBACK');
        console.error("Approval Error:", err);
        res.status(500).json({ error: err.message });
    } finally {
        client.release();
    }
});


// --- INITIALIZE BOT & START SERVER ---
whatsappClient.initialize();

app.listen(PORT, () => {
    console.log(`🚀 MiddleMan Server & Admin API running on port ${PORT}`);
});