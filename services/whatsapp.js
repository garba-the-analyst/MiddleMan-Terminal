import pkg from 'whatsapp-web.js';
const { Client, LocalAuth } = pkg;
import qrcode from 'qrcode-terminal';
import pool from '../config/db.js';
import { parseUserMessage } from './aiParser.js';
import { processWhatsAppImage } from './mediaHandler.js';
import { getCryptoPrice } from './cryptoApi.js';
// import { buyAirtime, withdrawFiat } from './fiatApi.js'; // Uncomment when connecting real APIs

export const whatsappClient = new Client({
    authStrategy: new LocalAuth(),
    puppeteer: { 
        args: [
            '--no-sandbox', 
            '--disable-setuid-sandbox',
            '--disable-dev-shm-usage', // CRUCIAL: Prevents RAM crashes in cloud environments
            '--disable-accelerated-2d-canvas',
            '--no-first-run',
            '--no-zygote',
            '--single-process', // Forces Chrome to use less memory
            '--disable-gpu'
        ] 
    }
});

whatsappClient.on('qr', (qr) => {
    console.log('\n======================================================');
    console.log('⚠️ TERMINAL QR DISTORTED? USE THE RAW STRING BELOW ⚠️');
    console.log('1. Copy the long text string below.');
    console.log('2. Go to https://www.qr-code-generator.com/');
    console.log('3. Select the "Text" option and paste the string.');
    console.log('4. Scan the QR code it generates on your screen.');
    console.log('======================================================\n');
    
    console.log('--- RAW QR STRING START ---');
    console.log(qr);
    console.log('--- RAW QR STRING END ---\n');

    // We will still try to print the graphical one just in case Render cooperates
    qrcode.generate(qr, { small: true });
});

whatsappClient.on('ready', () => {
    console.log('✅ MiddleMan Terminal Online (Secured + Utility + Withdrawals)!');
});

const CEX_TAKER_FEE = 0.001; 
const DEX_SWAP_FEE = 0.003;  

function calculateLiquidationPrice(entry, leverage, side) {
    let liq = side === 'LONG' ? entry - (entry / leverage) : entry + (entry / leverage);
    return liq <= 0 ? 0.0001 : liq; 
}

async function closePositionLogic(dbClient, userId, asset, message) {
    const posRes = await dbClient.query("SELECT * FROM active_positions WHERE user_id = $1 AND asset = $2 AND status = 'OPEN' LIMIT 1", [userId, asset]);
    if (posRes.rows.length === 0) return message.reply(`❌ You have no open positions for ${asset}.`);

    const pos = posRes.rows[0];
    const liveData = await getCryptoPrice(asset);
    if (!liveData.found) return message.reply("❌ Could not fetch real-time price.");

    const currentPrice = parseFloat(liveData.priceUsd);
    const entry = parseFloat(pos.entry_price);
    const leverage = parseFloat(pos.leverage);
    const margin = parseFloat(pos.margin_usd);
    
    let priceDiffPercent = (currentPrice - entry) / entry;
    if (pos.side === 'SHORT') priceDiffPercent = -priceDiffPercent;
    
    let grossPnlUsd = (margin * priceDiffPercent) * leverage;
    const closingFee = (margin * leverage) * CEX_TAKER_FEE;
    let netPnl = grossPnlUsd - closingFee;
    let totalReturn = margin + netPnl;

    let isLiquidated = false;
    if (totalReturn <= 0) {
        totalReturn = 0;
        netPnl = -margin;
        isLiquidated = true;
    }

    try {
        await dbClient.query('BEGIN');
        await dbClient.query("UPDATE active_positions SET status = 'CLOSED' WHERE id = $1", [pos.id]);
        if (totalReturn > 0) {
            await dbClient.query("UPDATE wallets SET balance = balance + $1 WHERE user_id = $2 AND currency = 'USDT' AND wallet_type = 'CEX'", [totalReturn, userId]);
        }
        await dbClient.query('COMMIT');
        
        const icon = netPnl >= 0 ? "🟩" : "🟥";
        let replyMsg = `💳 *Position Closed*\n\n*Trade:* ${pos.side} ${leverage}x ${pos.asset}\n*Entry:* $${entry.toFixed(4)}\n*Exit:* $${currentPrice.toFixed(4)}\n\n`;
        if (isLiquidated) replyMsg += `☠️ *LIQUIDATED*\nLost entire margin of $${margin}.`;
        else replyMsg += `*Margin Returned:* $${margin.toFixed(2)}\n*Net PnL:* ${icon} *$${netPnl.toFixed(2)}*\n*Added to CEX Wallet:* $${totalReturn.toFixed(2)}`;
        return message.reply(replyMsg);
    } catch (err) {
        await dbClient.query('ROLLBACK');
        return message.reply("❌ Failed to close position.");
    }
}

function formatWhatsAppNumber(phone) {
    let cleanPhone = phone.replace(/\D/g, ''); 
    if (cleanPhone.startsWith('0')) cleanPhone = '234' + cleanPhone.substring(1); 
    if (!cleanPhone.endsWith('@c.us')) cleanPhone += '@c.us';
    return cleanPhone;
}

whatsappClient.on('message', async (message) => {
    if (message.from === 'status@broadcast' || message.isGroupMsg) return;

    const senderNumber = message.from; 
    const textBody = message.body || "";
    const rawText = textBody.trim();
    const dbClient = await pool.connect();

    try {
        const chat = await message.getChat();
        await chat.sendStateTyping();

        let isNewUser = false;
        let userResult = await dbClient.query('SELECT * FROM users WHERE whatsapp_number = $1', [senderNumber]);
        
        if (userResult.rows.length === 0) {
            isNewUser = true;
            await dbClient.query('BEGIN');
            const insertUser = await dbClient.query("INSERT INTO users (whatsapp_number, current_state, state_data) VALUES ($1, 'AWAITING_NAME', '{}') RETURNING *", [senderNumber]);
            await dbClient.query('COMMIT');
            
            userResult = { rows: [insertUser.rows[0]] };
            return message.reply("👋 Welcome to *MiddleMan*! The ultimate Crypto & Fiat ecosystem.\n\nBefore we set up your wallets, let's get you registered.\n\n*What is your full name?*");
        }
        
        let user = userResult.rows[0];

        // --- REGISTRATION FLOW ---
        if (user.current_state === 'AWAITING_NAME') {
            const name = rawText;
            if (name.length < 2) return message.reply("Please enter a valid name.");
            await dbClient.query("UPDATE users SET current_state = 'AWAITING_PIN', state_data = $1 WHERE id = $2", [JSON.stringify({ name: name }), user.id]);
            return message.reply(`Nice to meet you, ${name}! 🤝\n\nTo secure your funds, please set a 4-digit Transaction PIN. You will need this to authorize withdrawals and transfers.\n\n*Reply with exactly 4 numbers (e.g., 1234):*`);
        }

        if (user.current_state === 'AWAITING_PIN') {
            const pin = rawText;
            if (!/^\d{4}$/.test(pin)) return message.reply("❌ Invalid PIN. Please reply with exactly 4 numbers (e.g., 1234).");
            
            const name = user.state_data.name || "Trader";
            try {
                await dbClient.query('BEGIN');
                await dbClient.query("UPDATE users SET current_state = 'IDLE', state_data = '{}', full_name = $1, pin = $2 WHERE id = $3", [name, pin, user.id]);
                // Airdrop 1000 USDT and 50,000 NGN
                await dbClient.query(`INSERT INTO wallets (user_id, wallet_type, currency, balance) VALUES ($1, 'CEX', 'USDT', 1000.00), ($1, 'DEX', 'USDT', 0.00), ($1, 'FIAT', 'NGN', 50000.00)`, [user.id]);
                await dbClient.query('COMMIT');
                return message.reply(`✅ *Registration Complete!*\n\nYour 4-digit PIN is set. Keep it safe!\n\n🎉 I've deposited a mock **$1,000 USDT** and **₦50,000** into your wallets to test the ecosystem.\n\nType /help to see what you can do.`);
            } catch (err) {
                await dbClient.query('ROLLBACK');
                return message.reply("❌ An error occurred during registration. Please try again.");
            }
        }

        // --- SECURITY INTERCEPTORS (PIN AUTHORIZATION) ---
        if (user.current_state.startsWith('AWAITING_PIN_')) {
            if (rawText.toLowerCase() === 'cancel') {
                await dbClient.query("UPDATE users SET current_state = 'IDLE', state_data = '{}' WHERE id = $1", [user.id]);
                return message.reply("Action cancelled.");
            }
            if (rawText !== user.pin) return message.reply("❌ Incorrect PIN. Please try again, or type 'cancel' to abort.");

            const txData = user.state_data;

            try {
                await dbClient.query('BEGIN');

                if (user.current_state === 'AWAITING_PIN_SEND') {
                    const { amount, currency, wallet_type, recipient_phone } = txData;
                    await dbClient.query("UPDATE wallets SET balance = balance - $1 WHERE user_id = $2 AND currency = $3 AND wallet_type = $4", [amount, user.id, currency, wallet_type]);
                    const recipientRes = await dbClient.query("SELECT id, full_name FROM users WHERE whatsapp_number = $1", [recipient_phone]);
                    const recipientId = recipientRes.rows[0].id;
                    await dbClient.query(`INSERT INTO wallets (user_id, wallet_type, currency, balance) VALUES ($1, $2, $3, $4) ON CONFLICT (user_id, currency, wallet_type) DO UPDATE SET balance = wallets.balance + EXCLUDED.balance`, [recipientId, wallet_type, currency, amount]);
                    await dbClient.query('COMMIT');
                    await dbClient.query("UPDATE users SET current_state = 'IDLE', state_data = '{}' WHERE id = $1", [user.id]);
                    
                    const symbol = currency === 'NGN' ? '₦' : '$';
                    message.reply(`✅ *Transfer Successful!*\nYou sent ${symbol}${amount.toLocaleString()} ${currency} to ${recipientRes.rows[0].full_name}.`);
                    try { await whatsappClient.sendMessage(recipient_phone, `🔔 *You received funds!*\n\n${user.full_name} just sent you ${symbol}${amount.toLocaleString()} ${currency} to your ${wallet_type} wallet!\n\nType /bal to check.`); } catch (e) {}
                    return;
                }

                if (user.current_state === 'AWAITING_PIN_BRIDGE') {
                    const { amount, currency, from, to } = txData;
                    await dbClient.query("UPDATE wallets SET balance = balance - $1 WHERE user_id = $2 AND currency = $3 AND wallet_type = $4", [amount, user.id, currency, from]);
                    await dbClient.query(`INSERT INTO wallets (user_id, wallet_type, currency, balance) VALUES ($1, $2, $3, $4) ON CONFLICT (user_id, currency, wallet_type) DO UPDATE SET balance = wallets.balance + EXCLUDED.balance`, [user.id, to, currency, amount]);
                    await dbClient.query('COMMIT');
                    await dbClient.query("UPDATE users SET current_state = 'IDLE', state_data = '{}' WHERE id = $1", [user.id]);
                    return message.reply(`✅ *Bridge Successful!*\nMoved ${amount.toLocaleString()} ${currency} from ${from} to ${to}.`);
                }

                if (user.current_state === 'AWAITING_PIN_AIRTIME') {
                    const { amount, network, phone } = txData;
                    await dbClient.query("UPDATE wallets SET balance = balance - $1 WHERE user_id = $2 AND currency = 'NGN' AND wallet_type = 'FIAT'", [amount, user.id]);
                    await dbClient.query('COMMIT');
                    await dbClient.query("UPDATE users SET current_state = 'IDLE', state_data = '{}' WHERE id = $1", [user.id]);
                    return message.reply(`📱 *Airtime Purchase Successful!*\n\nSent ₦${amount.toLocaleString()} ${network} airtime to ${phone}.`);
                }

                if (user.current_state === 'AWAITING_PIN_DATA') {
                    const { amount, network, phone } = txData;
                    await dbClient.query("UPDATE wallets SET balance = balance - $1 WHERE user_id = $2 AND currency = 'NGN' AND wallet_type = 'FIAT'", [amount, user.id]);
                    await dbClient.query('COMMIT');
                    await dbClient.query("UPDATE users SET current_state = 'IDLE', state_data = '{}' WHERE id = $1", [user.id]);
                    return message.reply(`📶 *Data Bundle Activated!*\n\nAllocated ₦${amount.toLocaleString()} ${network} data plan to ${phone}.`);
                }

                if (user.current_state === 'AWAITING_PIN_WITHDRAW_FIAT') {
                    const { amount, bank_name, account_number } = txData;
                    await dbClient.query("UPDATE wallets SET balance = balance - $1 WHERE user_id = $2 AND currency = 'NGN' AND wallet_type = 'FIAT'", [amount, user.id]);
                    await dbClient.query('COMMIT');
                    await dbClient.query("UPDATE users SET current_state = 'IDLE', state_data = '{}' WHERE id = $1", [user.id]);
                    return message.reply(`🏦 *Bank Withdrawal Initiated!*\n\n₦${amount.toLocaleString()} is being processed to ${bank_name} (Acc: ${account_number}).\nIt should arrive shortly.`);
                }

                if (user.current_state === 'AWAITING_PIN_WITHDRAW_CRYPTO') {
                    const { amount, coin, network, address } = txData;
                    await dbClient.query("UPDATE wallets SET balance = balance - $1 WHERE user_id = $2 AND currency = $3 AND wallet_type = 'CEX'", [amount, user.id, coin]);
                    await dbClient.query('COMMIT');
                    await dbClient.query("UPDATE users SET current_state = 'IDLE', state_data = '{}' WHERE id = $1", [user.id]);
                    return message.reply(`⛓️ *Crypto Withdrawal Processing!*\n\nSending ${amount} ${coin} via ${network} network to:\n\`${address}\`\n\nTrack progress on the blockchain explorer.`);
                }

            } catch (err) {
                await dbClient.query('ROLLBACK');
                return message.reply("❌ Transaction failed at the database level.");
            }
        }

        // --- PENDING PHOTO UPLOADS ---
        if (user.current_state === 'AWAITING_GIFTCARD_IMAGE') {
            if (message.hasMedia) {
                await message.reply("Uploading your card securely. Please wait a moment...");
                const media = await message.downloadMedia();
                const imageUrl = await processWhatsAppImage(media.data, media.mimetype);
                const cardData = user.state_data; 
                await dbClient.query(`INSERT INTO gift_card_trades (user_id, card_category, country, card_type, amount_in_usd, offered_naira_value, image_url, status) VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending')`, [user.id, cardData.brand, cardData.country || 'US', cardData.format || 'physical', cardData.amount, cardData.amount * 1250, imageUrl]);
                await dbClient.query("UPDATE users SET current_state = 'IDLE', state_data = '{}' WHERE id = $1", [user.id]);
                return message.reply("✅ Image received! Your trade is pending review.");
            } else if (rawText.toLowerCase() === 'cancel') {
                await dbClient.query("UPDATE users SET current_state = 'IDLE', state_data = '{}' WHERE id = $1", [user.id]);
                return message.reply("Trade cancelled.");
            } else {
                return message.reply("Please upload the image, or type 'cancel' to stop.");
            }
        }

        // --- COMMAND ROUTER ---
        if (message.type === 'chat' && rawText.startsWith('/')) {
            if (user.current_state === 'AWAITING_AI_INFO') await dbClient.query("UPDATE users SET current_state = 'IDLE', state_data = '{}' WHERE id = $1", [user.id]);
            const args = rawText.split(' ');
            const command = args[0].toLowerCase();

            switch (command) {
                case '/help':
                    return message.reply("🤖 *MiddleMan Terminal*\n\n💰 *Transfers:*\n/bal - View Balances\n/send [amount] [coin] [phone]\n/bridge [amount] [coin] [from] [to]\n\n🌍 *Utility & Withdraw:*\n/airtime [network] [amount] [phone]\n/data [network] [amount] [phone]\n/withdraw_fiat [amount] [bank] [account]\n/withdraw_crypto [amount] [coin] [network] [address]\n\n📈 *CEX Trading:*\n/spot [coin] [usd]\n/long [asset] [lev] [margin]\n/positions\n/tp [asset] [price]\n/close [asset]\n\n🦊 *DEX Trading:*\n/dex [contract] [usd]\n\n💳 *Giftcards:*\n/gc - Trade Giftcard");
                
                case '/bal':
                case '/balance':
                    const walletRes = await dbClient.query("SELECT wallet_type, currency, balance FROM wallets WHERE user_id = $1 AND balance > 0 ORDER BY wallet_type ASC, balance DESC", [user.id]);
                    let balMsg = `📊 *Portfolio for ${user.full_name}*\n\n`;
                    const cexWallets = walletRes.rows.filter(w => w.wallet_type === 'CEX');
                    const dexWallets = walletRes.rows.filter(w => w.wallet_type === 'DEX');
                    const fiatWallets = walletRes.rows.filter(w => w.wallet_type === 'FIAT');

                    balMsg += "🏦 *CEX Wallet:*\n";
                    if (cexWallets.length === 0) balMsg += "Empty\n";
                    else cexWallets.forEach(w => balMsg += `- ${w.currency}: ${w.currency==='NGN'?'₦':'$'}${parseFloat(w.balance).toLocaleString()}\n`);

                    balMsg += "\n🦊 *DEX Wallet:*\n";
                    if (dexWallets.length === 0) balMsg += "Empty\n";
                    else dexWallets.forEach(w => balMsg += `- ${w.currency}: $${parseFloat(w.balance).toLocaleString()}\n`);

                    balMsg += "\n🇳🇬 *FIAT Wallet:*\n";
                    if (fiatWallets.length === 0) balMsg += "Empty\n";
                    else fiatWallets.forEach(w => balMsg += `- ${w.currency}: ₦${parseFloat(w.balance).toLocaleString()}\n`);
                    return message.reply(balMsg);

                // --- UTILITY & WITHDRAWALS ---
                case '/airtime':
                    if (args.length < 4) return message.reply("Usage: /airtime [network] [amount] [phone]\nExample: /airtime MTN 1000 08123456789");
                    const aNetwork = args[1].toUpperCase();
                    const aAmount = parseFloat(args[2]);
                    const aPhone = args[3];

                    if (isNaN(aAmount) || aAmount <= 0) return message.reply("❌ Invalid amount.");
                    
                    const aFiatRes = await dbClient.query("SELECT balance FROM wallets WHERE user_id = $1 AND currency = 'NGN' AND wallet_type = 'FIAT'", [user.id]);
                    const aFiatBal = aFiatRes.rows.length > 0 ? parseFloat(aFiatRes.rows[0].balance) : 0;
                    if (aFiatBal < aAmount) return message.reply(`❌ Insufficient FIAT funds. You have ₦${aFiatBal.toLocaleString()}.`);

                    await dbClient.query("UPDATE users SET current_state = 'AWAITING_PIN_AIRTIME', state_data = $1 WHERE id = $2", [JSON.stringify({ amount: aAmount, network: aNetwork, phone: aPhone }), user.id]);
                    return message.reply(`🔒 *Airtime Authorization*\n\nYou are buying **₦${aAmount} ${aNetwork} Airtime** for **${aPhone}**.\n\nPlease reply with your 4-digit PIN to confirm.`);

                case '/data':
                    if (args.length < 4) return message.reply("Usage: /data [network] [amount_in_naira] [phone]\nExample: /data AIRTEL 5000 08123456789");
                    const dNetwork = args[1].toUpperCase();
                    const dAmount = parseFloat(args[2]);
                    const dPhone = args[3];

                    if (isNaN(dAmount) || dAmount <= 0) return message.reply("❌ Invalid amount.");
                    
                    const dFiatRes = await dbClient.query("SELECT balance FROM wallets WHERE user_id = $1 AND currency = 'NGN' AND wallet_type = 'FIAT'", [user.id]);
                    const dFiatBal = dFiatRes.rows.length > 0 ? parseFloat(dFiatRes.rows[0].balance) : 0;
                    if (dFiatBal < dAmount) return message.reply(`❌ Insufficient FIAT funds. You have ₦${dFiatBal.toLocaleString()}.`);

                    await dbClient.query("UPDATE users SET current_state = 'AWAITING_PIN_DATA', state_data = $1 WHERE id = $2", [JSON.stringify({ amount: dAmount, network: dNetwork, phone: dPhone }), user.id]);
                    return message.reply(`🔒 *Data Bundle Authorization*\n\nYou are buying a **₦${dAmount} ${dNetwork} Data Plan** for **${dPhone}**.\n\nPlease reply with your 4-digit PIN to confirm.`);

                case '/withdraw_fiat':
                    if (args.length < 4) return message.reply("Usage: /withdraw_fiat [amount] [bank] [account]\nExample: /withdraw_fiat 5000 Palmpay 08123456789");
                    const wFiatAmount = parseFloat(args[1]);
                    const bankName = args[2];
                    const accNumber = args[3];

                    if (isNaN(wFiatAmount) || wFiatAmount <= 0) return message.reply("❌ Invalid amount.");
                    
                    const wFiatBalRes = await dbClient.query("SELECT balance FROM wallets WHERE user_id = $1 AND currency = 'NGN' AND wallet_type = 'FIAT'", [user.id]);
                    const wFiatBal = wFiatBalRes.rows.length > 0 ? parseFloat(wFiatBalRes.rows[0].balance) : 0;
                    if (wFiatBal < wFiatAmount) return message.reply(`❌ Insufficient FIAT funds. You have ₦${wFiatBal.toLocaleString()}.`);

                    await dbClient.query("UPDATE users SET current_state = 'AWAITING_PIN_WITHDRAW_FIAT', state_data = $1 WHERE id = $2", [JSON.stringify({ amount: wFiatAmount, bank_name: bankName, account_number: accNumber }), user.id]);
                    return message.reply(`🔒 *Bank Withdrawal Authorization*\n\nWithdrawing **₦${wFiatAmount.toLocaleString()}** to **${bankName}** (Acc: ${accNumber}).\n\nPlease reply with your 4-digit PIN to confirm.`);

                case '/withdraw_crypto':
                    if (args.length < 5) return message.reply("Usage: /withdraw_crypto [amount] [coin] [network] [address]\nExample: /withdraw_crypto 50 USDT TRC20 Txyz...");
                    const wCryptAmount = parseFloat(args[1]);
                    const wCryptCoin = args[2].toUpperCase();
                    const wCryptNetwork = args[3].toUpperCase();
                    const wCryptAddress = args[4];

                    if (isNaN(wCryptAmount) || wCryptAmount <= 0) return message.reply("❌ Invalid amount.");
                    
                    const wCryptBalRes = await dbClient.query("SELECT balance FROM wallets WHERE user_id = $1 AND currency = $2 AND wallet_type = 'CEX'", [user.id, wCryptCoin]);
                    const wCryptBal = wCryptBalRes.rows.length > 0 ? parseFloat(wCryptBalRes.rows[0].balance) : 0;
                    if (wCryptBal < wCryptAmount) return message.reply(`❌ Insufficient ${wCryptCoin} in your CEX wallet. You have ${wCryptBal.toLocaleString()}.`);

                    await dbClient.query("UPDATE users SET current_state = 'AWAITING_PIN_WITHDRAW_CRYPTO', state_data = $1 WHERE id = $2", [JSON.stringify({ amount: wCryptAmount, coin: wCryptCoin, network: wCryptNetwork, address: wCryptAddress }), user.id]);
                    return message.reply(`🔒 *Crypto Withdrawal Authorization*\n\nSending **${wCryptAmount} ${wCryptCoin}** via **${wCryptNetwork}** to:\n\`${wCryptAddress}\`\n\nPlease reply with your 4-digit PIN to confirm.`);

                // --- TRANSFERS & TRADING ---
                case '/send':
                    if (args.length < 4) return message.reply("Usage: /send [amount] [currency] [phone]");
                    const sendAmount = parseFloat(args[1]);
                    const sendCurrency = args[2].toUpperCase();
                    const rawRecipientNumber = args[3];
                    if (isNaN(sendAmount) || sendAmount <= 0) return message.reply("❌ Invalid amount.");
                    const sendWalletType = sendCurrency === 'NGN' ? 'FIAT' : 'CEX';
                    const senderBalRes = await dbClient.query("SELECT balance FROM wallets WHERE user_id = $1 AND currency = $2 AND wallet_type = $3", [user.id, sendCurrency, sendWalletType]);
                    const senderBal = senderBalRes.rows.length > 0 ? parseFloat(senderBalRes.rows[0].balance) : 0;
                    if (senderBal < sendAmount) return message.reply(`❌ Insufficient funds. You have ${senderBal.toFixed(2)} ${sendCurrency}.`);
                    const formattedRecipient = formatWhatsAppNumber(rawRecipientNumber);
                    if (formattedRecipient === senderNumber) return message.reply("❌ You cannot send funds to yourself.");
                    const recipientCheck = await dbClient.query("SELECT full_name FROM users WHERE whatsapp_number = $1", [formattedRecipient]);
                    if (recipientCheck.rows.length === 0) return message.reply(`❌ User not found.`);
                    await dbClient.query("UPDATE users SET current_state = 'AWAITING_PIN_SEND', state_data = $1 WHERE id = $2", [JSON.stringify({ amount: sendAmount, currency: sendCurrency, wallet_type: sendWalletType, recipient_phone: formattedRecipient }), user.id]);
                    return message.reply(`🔒 *Transfer Authorization*\n\nSending **${sendAmount} ${sendCurrency}** to **${recipientCheck.rows[0].full_name}**.\n\nPlease reply with your 4-digit PIN.`);

                case '/bridge':
                    if (args.length < 5) return message.reply("Usage: /bridge [amount] [currency] [from] [to]");
                    const bAmount = parseFloat(args[1]);
                    const bCur = args[2].toUpperCase();
                    const bFrom = args[3].toUpperCase();
                    const bTo = args[4].toUpperCase();
                    if (isNaN(bAmount) || bAmount <= 0) return message.reply("❌ Invalid amount.");
                    if (!['CEX', 'DEX'].includes(bFrom) || !['CEX', 'DEX'].includes(bTo) || bFrom === bTo) return message.reply("❌ Invalid wallets.");
                    const bridgeBalRes = await dbClient.query("SELECT balance FROM wallets WHERE user_id = $1 AND currency = $2 AND wallet_type = $3", [user.id, bCur, bFrom]);
                    const bridgeBal = bridgeBalRes.rows.length > 0 ? parseFloat(bridgeBalRes.rows[0].balance) : 0;
                    if (bridgeBal < bAmount) return message.reply(`❌ Insufficient funds in ${bFrom}.`);
                    await dbClient.query("UPDATE users SET current_state = 'AWAITING_PIN_BRIDGE', state_data = $1 WHERE id = $2", [JSON.stringify({ amount: bAmount, currency: bCur, from: bFrom, to: bTo }), user.id]);
                    return message.reply(`🔒 *Bridge Authorization*\n\nMoving **${bAmount} ${bCur}** from ${bFrom} to ${bTo}.\n\nPlease reply with your 4-digit PIN.`);

                case '/spot':
                    if (args.length < 3) return message.reply("Usage: /spot [coin] [usd_amount]");
                    const spotCoin = args[1].toUpperCase();
                    const spotSpend = parseFloat(args[2]);
                    const spotUsdtRes = await dbClient.query("SELECT balance FROM wallets WHERE user_id = $1 AND currency = 'USDT' AND wallet_type = 'CEX'", [user.id]);
                    const spotUsdt = spotUsdtRes.rows.length > 0 ? parseFloat(spotUsdtRes.rows[0].balance) : 0;
                    if (spotUsdt < spotSpend) return message.reply(`❌ Insufficient CEX funds.`);
                    await message.reply(`⚡ Buying ${spotCoin} on Spot Market...`);
                    const spotData = await getCryptoPrice(spotCoin);
                    if (!spotData.found) return message.reply(`❌ Could not find reliable data.`);
                    const spotTokens = (spotSpend / parseFloat(spotData.priceUsd)).toFixed(6);
                    try {
                        await dbClient.query('BEGIN');
                        await dbClient.query("UPDATE wallets SET balance = balance - $1 WHERE user_id = $2 AND currency = 'USDT' AND wallet_type = 'CEX'", [spotSpend, user.id]);
                        await dbClient.query(`INSERT INTO wallets (user_id, wallet_type, currency, balance) VALUES ($1, 'CEX', $2, $3) ON CONFLICT (user_id, currency, wallet_type) DO UPDATE SET balance = wallets.balance + EXCLUDED.balance`, [user.id, spotData.symbol, spotTokens]);
                        await dbClient.query('COMMIT');
                        return message.reply(`✅ *CEX Spot Buy Successful*\nAdded ${Number(spotTokens).toLocaleString()} ${spotData.symbol} to CEX Wallet.`);
                    } catch (err) { await dbClient.query('ROLLBACK'); return message.reply("❌ Transaction failed."); }

                case '/long':
                case '/short':
                    if (args.length < 4) return message.reply(`Usage: ${command} [asset] [leverage] [margin]`);
                    const asset = args[1].toUpperCase();
                    const leverage = parseFloat(args[2].replace(/x/i, ''));
                    const margin = parseFloat(args[3]);
                    if (isNaN(leverage) || isNaN(margin) || margin <= 0) return message.reply("❌ Leverage and margin must be numbers.");
                    const existingRes = await dbClient.query("SELECT id FROM active_positions WHERE user_id = $1 AND asset = $2 AND status = 'OPEN'", [user.id, asset]);
                    if (existingRes.rows.length > 0) return message.reply(`❌ You already have an open position on ${asset}.`);
                    const positionSizeUsd = margin * leverage;
                    const requiredUsdt = margin + (positionSizeUsd * CEX_TAKER_FEE);
                    const futuresUsdtRes = await dbClient.query("SELECT balance FROM wallets WHERE user_id = $1 AND currency = 'USDT' AND wallet_type = 'CEX'", [user.id]);
                    const futuresUsdt = futuresUsdtRes.rows.length > 0 ? parseFloat(futuresUsdtRes.rows[0].balance) : 0;
                    if (futuresUsdt < requiredUsdt) return message.reply(`❌ Insufficient funds. Need $${requiredUsdt.toFixed(2)} USDT.`);
                    await message.reply(`⏱️ Analyzing market data...`);
                    const cexData = await getCryptoPrice(asset);
                    if (!cexData.found) return message.reply(`❌ Could not fetch price.`);
                    const side = command === '/long' ? 'LONG' : 'SHORT';
                    const entryPrice = parseFloat(cexData.priceUsd);
                    const tokensContracts = (positionSizeUsd / entryPrice).toFixed(6);
                    try {
                        await dbClient.query('BEGIN');
                        await dbClient.query("UPDATE wallets SET balance = balance - $1 WHERE user_id = $2 AND currency = 'USDT' AND wallet_type = 'CEX'", [requiredUsdt, user.id]);
                        await dbClient.query(`INSERT INTO active_positions (user_id, asset, side, leverage, margin_usd, entry_price, position_size_tokens) VALUES ($1, $2, $3, $4, $5, $6, $7)`, [user.id, cexData.symbol, side, leverage, margin, entryPrice, tokensContracts]);
                        await dbClient.query('COMMIT');
                        return message.reply(`✅ *${side} Opened*\n*Pair:* ${cexData.symbol}/USDT\n*Entry:* $${entryPrice.toFixed(4)}\n*Lev:* ${leverage}x`);
                    } catch (err) { await dbClient.query('ROLLBACK'); return message.reply("❌ Failed to open position."); }

                case '/tp':
                case '/sl':
                    if (args.length < 3) return message.reply(`Usage: ${command} [asset] [price]`);
                    const tAsset = args[1].toUpperCase();
                    const tPrice = parseFloat(args[2]);
                    if (isNaN(tPrice)) return message.reply("❌ Price must be a number.");
                    const posCheck = await dbClient.query("SELECT id FROM active_positions WHERE user_id = $1 AND asset = $2 AND status = 'OPEN'", [user.id, tAsset]);
                    if (posCheck.rows.length === 0) return message.reply(`❌ No open position found for ${tAsset}.`);
                    const col = command === '/tp' ? 'take_profit' : 'stop_loss';
                    await dbClient.query(`UPDATE active_positions SET ${col} = $1 WHERE id = $2`, [tPrice, posCheck.rows[0].id]);
                    return message.reply(`✅ *Limits Set!*\n${tAsset} will trigger at $${tPrice.toLocaleString()}.`);

                case '/positions':
                    const posRes = await dbClient.query("SELECT * FROM active_positions WHERE user_id = $1 AND status = 'OPEN'", [user.id]);
                    if (posRes.rows.length === 0) return message.reply("You have no open positions.");
                    let posMsg = "*Your Active Trades*\n\n";
                    for (const pos of posRes.rows) {
                        const liveData = await getCryptoPrice(pos.asset);
                        const currentPrice = parseFloat(liveData.priceUsd);
                        const entry = parseFloat(pos.entry_price);
                        const lev = parseFloat(pos.leverage);
                        let priceDiffPercent = (currentPrice - entry) / entry;
                        if (pos.side === 'SHORT') priceDiffPercent = -priceDiffPercent;
                        const grossPnl = (parseFloat(pos.margin_usd) * priceDiffPercent) * lev;
                        const icon = grossPnl >= 0 ? "🟩" : "🟥";
                        posMsg += `${icon} *${pos.side} ${lev}x ${pos.asset}*\nEntry: $${entry.toFixed(4)} | Live: $${currentPrice.toFixed(4)}\nGross PnL: *$${grossPnl.toFixed(2)}*\n\n`;
                    }
                    return message.reply(posMsg);

                case '/close':
                    if (args.length < 2) return message.reply("Usage: /close [asset]");
                    return await closePositionLogic(dbClient, user.id, args[1].toUpperCase(), message);

                case '/dex':
                case '/buy':
                    if (args.length < 3) return message.reply("Usage: /dex [contract] [usd]");
                    const dexCoin = args[1];
                    const dexSpend = parseFloat(args[2]);
                    const dexUsdtRes = await dbClient.query("SELECT balance FROM wallets WHERE user_id = $1 AND currency = 'USDT' AND wallet_type = 'CEX'", [user.id]);
                    const dexUsdt = dexUsdtRes.rows.length > 0 ? parseFloat(dexUsdtRes.rows[0].balance) : 0;
                    if (dexUsdt < dexSpend) return message.reply(`❌ Insufficient funds for DEX Swap.`);
                    await message.reply(`🦊 Routing swap through Web3...`);
                    const dexData = await getCryptoPrice(dexCoin);
                    if (!dexData.found) return message.reply(`❌ Could not locate pool.`);
                    const dexTokens = ((dexSpend - (dexSpend * DEX_SWAP_FEE)) / parseFloat(dexData.priceUsd)).toFixed(4);
                    try {
                        await dbClient.query('BEGIN');
                        await dbClient.query("UPDATE wallets SET balance = balance - $1 WHERE user_id = $2 AND currency = 'USDT' AND wallet_type = 'CEX'", [dexSpend, user.id]);
                        await dbClient.query(`INSERT INTO wallets (user_id, wallet_type, currency, balance) VALUES ($1, 'DEX', $2, $3) ON CONFLICT (user_id, currency, wallet_type) DO UPDATE SET balance = wallets.balance + EXCLUDED.balance`, [user.id, dexData.symbol, dexTokens]);
                        await dbClient.query('COMMIT');
                        return message.reply(`✅ *DEX Swap Successful*\nAdded ${Number(dexTokens).toLocaleString()} ${dexData.symbol} to your DEX Wallet.`);
                    } catch (err) { await dbClient.query('ROLLBACK'); return message.reply("❌ Transaction failed."); }

                case '/price':
                    if (args.length < 2) return message.reply("Usage: /price [coin]");
                    const priceData = await getCryptoPrice(args[1]);
                    if (!priceData.found) return message.reply(`❌ Could not find reliable data.`);
                    const trend = priceData.change24h >= 0 ? "🟩" : "🟥";
                    return whatsappClient.sendMessage(message.from, `*${priceData.name} (${priceData.symbol})*\n\n💵 *Price:* $${priceData.priceUsd}\n${trend} *24h Change:* ${priceData.change24h}%\n🏦 *Market:* ${priceData.dex}\n📝 *Contract:* ${priceData.contract}\n\n📈 *Live Chart:* ${priceData.url}`, { linkPreview: true });

                case '/gc':
                case '/giftcard':
                    await dbClient.query("UPDATE users SET current_state = 'AWAITING_AI_INFO', state_data = $1 WHERE id = $2", [JSON.stringify({ intent: "TRADE_GIFTCARD", giftcard_details: { card_brand: "Unknown", amount: 0, format: "" } }), user.id]);
                    return message.reply("💳 Let's trade a gift card. What brand and amount are you selling?");

                default:
                    return message.reply("Unknown command. Type /help.");
            }
        }

        // --- AI FALLBACK ---
        if (message.type === 'chat' && !rawText.startsWith('/')) {
            const isAnsweringAi = user.current_state === 'AWAITING_AI_INFO';

            if (isAnsweringAi && rawText.toLowerCase() === 'cancel') {
                await dbClient.query("UPDATE users SET current_state = 'IDLE', state_data = '{}' WHERE id = $1", [user.id]);
                return message.reply("Request cancelled.");
            }

            if (!isAnsweringAi) {
                const greetings = ['hi', 'hello', 'hey', 'sup', 'gm'];
                if (greetings.includes(rawText.toLowerCase())) {
                    return message.reply(`Hello ${user.full_name}! 👋 How can I help you today?`);
                }
            }

            const partialStateData = isAnsweringAi ? user.state_data : {};
            const parsedCommand = await parseUserMessage(textBody, partialStateData);

            if (parsedCommand.missing_information && parsedCommand.missing_information !== "none") {
                await dbClient.query("UPDATE users SET current_state = 'AWAITING_AI_INFO', state_data = $1 WHERE id = $2", [JSON.stringify(parsedCommand), user.id]);
                return message.reply(parsedCommand.missing_information);
            }

            if (isAnsweringAi) await dbClient.query("UPDATE users SET current_state = 'IDLE', state_data = '{}' WHERE id = $1", [user.id]);

            switch (parsedCommand.intent) {
                case "BUY_AIRTIME":
                    const { network, amount, phone } = parsedCommand;
                    const fiatBalRes = await dbClient.query("SELECT balance FROM wallets WHERE user_id = $1 AND currency = 'NGN' AND wallet_type = 'FIAT'", [user.id]);
                    const fiatBal = fiatBalRes.rows.length > 0 ? parseFloat(fiatBalRes.rows[0].balance) : 0;
                    if (fiatBal < amount) return message.reply(`❌ Insufficient FIAT funds. You have ₦${fiatBal.toLocaleString()}.`);
                    await dbClient.query("UPDATE users SET current_state = 'AWAITING_PIN_AIRTIME', state_data = $1 WHERE id = $2", [JSON.stringify({ amount: amount, network: network, phone: phone }), user.id]);
                    return message.reply(`🔒 *Airtime Authorization*\n\nYou are buying **₦${amount} ${network} Airtime** for **${phone}**.\n\nPlease reply with your 4-digit PIN to confirm.`);

                case "BUY_DATA":
                    const { network: dNet, amount: dAmt, phone: dPh } = parsedCommand;
                    const dataBalRes = await dbClient.query("SELECT balance FROM wallets WHERE user_id = $1 AND currency = 'NGN' AND wallet_type = 'FIAT'", [user.id]);
                    const dataBal = dataBalRes.rows.length > 0 ? parseFloat(dataBalRes.rows[0].balance) : 0;
                    if (dataBal < dAmt) return message.reply(`❌ Insufficient FIAT funds. You have ₦${dataBal.toLocaleString()}.`);
                    await dbClient.query("UPDATE users SET current_state = 'AWAITING_PIN_DATA', state_data = $1 WHERE id = $2", [JSON.stringify({ amount: dAmt, network: dNet, phone: dPh }), user.id]);
                    return message.reply(`🔒 *Data Bundle Authorization*\n\nYou are buying a **₦${dAmt} ${dNet} Data Plan** for **${dPh}**.\n\nPlease reply with your 4-digit PIN to confirm.`);

                case "WITHDRAW_FIAT":
                    const { bank_name, amount: fAmount, account_number } = parsedCommand;
                    const wFiatBalRes = await dbClient.query("SELECT balance FROM wallets WHERE user_id = $1 AND currency = 'NGN' AND wallet_type = 'FIAT'", [user.id]);
                    const wFiatBal = wFiatBalRes.rows.length > 0 ? parseFloat(wFiatBalRes.rows[0].balance) : 0;
                    if (wFiatBal < fAmount) return message.reply(`❌ Insufficient FIAT funds.`);
                    await dbClient.query("UPDATE users SET current_state = 'AWAITING_PIN_WITHDRAW_FIAT', state_data = $1 WHERE id = $2", [JSON.stringify({ amount: fAmount, bank_name: bank_name, account_number: account_number }), user.id]);
                    return message.reply(`🔒 *Bank Withdrawal Authorization*\n\nWithdrawing **₦${fAmount.toLocaleString()}** to **${bank_name}** (Acc: ${account_number}).\n\nPlease reply with your 4-digit PIN.`);

                case "WITHDRAW_CRYPTO":
                    const { coin, amount: cAmount, network: cNetwork, address } = parsedCommand;
                    const wCryptBalRes = await dbClient.query("SELECT balance FROM wallets WHERE user_id = $1 AND currency = $2 AND wallet_type = 'CEX'", [user.id, coin.toUpperCase()]);
                    const wCryptBal = wCryptBalRes.rows.length > 0 ? parseFloat(wCryptBalRes.rows[0].balance) : 0;
                    if (wCryptBal < cAmount) return message.reply(`❌ Insufficient ${coin.toUpperCase()} in your CEX wallet.`);
                    await dbClient.query("UPDATE users SET current_state = 'AWAITING_PIN_WITHDRAW_CRYPTO', state_data = $1 WHERE id = $2", [JSON.stringify({ amount: cAmount, coin: coin.toUpperCase(), network: cNetwork, address: address }), user.id]);
                    return message.reply(`🔒 *Crypto Withdrawal Authorization*\n\nSending **${cAmount} ${coin.toUpperCase()}** via **${cNetwork}** to:\n\`${address}\`\n\nPlease reply with your PIN.`);

                case "HELP":
                    return message.reply("🤖 *MiddleMan Commands*\nYou can use commands like `/data MTN 5000 0812...` or just text me naturally!");
                case "CHECK_BALANCE":
                    return message.reply("Type /bal to see your separated CEX, DEX, and FIAT wallets.");
                default:
                    return message.reply("I didn't quite catch that. Type /help to see my commands.");
            }
        }

    } catch (error) {
        console.error("Error:", error);
    } finally {
        dbClient.release();
    }
});