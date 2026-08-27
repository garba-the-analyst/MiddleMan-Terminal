import { GoogleGenerativeAI } from '@google/generative-ai';
import 'dotenv/config';

const genAI = new GoogleGenerativeAI(process.env.GEMINI_API_KEY);

export async function parseUserMessage(messageText, partialState = {}) {
    try {
        const model = genAI.getGenerativeModel({ model: "gemini-2.5-flash" });

        const prompt = `
        You are the intelligent intent parser for a fintech bot called MiddleMan.
        The user sent this message: "${messageText}"
        Previously extracted data (if any): ${JSON.stringify(partialState)}

        Extract the intent and respond STRICTLY with a valid JSON object matching one of these formats:

        1. CEX_SPOT_TRADE (Buying standard major crypto to hold)
        { "intent": "CEX_SPOT_TRADE", "coin": "BTC", "amount": 100, "missing_information": "none" }

        2. DEX_TRADE (Buying meme coins or interacting with Web3/contracts)
        { "intent": "DEX_TRADE", "coin": "WIF", "amount": 50, "missing_information": "none" }

        3. BUY_AIRTIME (Buying mobile network airtime/call credit)
        { "intent": "BUY_AIRTIME", "network": "MTN", "amount": 1000, "phone": "08012345678", "missing_information": "none" }

        4. BUY_DATA (Buying mobile internet data plans/bundles)
        { "intent": "BUY_DATA", "network": "AIRTEL", "amount": 5000, "phone": "08012345678", "missing_information": "none" }

        5. WITHDRAW_FIAT (Sending Naira to an external bank like Palmpay, Opay, GTB)
        { "intent": "WITHDRAW_FIAT", "bank_name": "Palmpay", "amount": 5000, "account_number": "08123456789", "missing_information": "none" }

        6. WITHDRAW_CRYPTO (Sending crypto to an external wallet)
        { "intent": "WITHDRAW_CRYPTO", "coin": "USDT", "amount": 50, "network": "TRC20", "address": "Txyz123...", "missing_information": "none" }

        7. SET_LIMITS (Setting Take Profit or Stop Loss on an existing trade)
        { "intent": "SET_LIMITS", "coin": "BTC", "take_profit": 65000, "stop_loss": 58000, "missing_information": "none" }

        8. CLOSE_POSITION (Exiting a trade)
        { "intent": "CLOSE_POSITION", "coin": "BTC", "missing_information": "none" }

        9. TRADE_GIFTCARD (Trading a gift card for Naira)
        { "intent": "TRADE_GIFTCARD", "giftcard_details": { "card_brand": "Apple", "amount": 50, "format": "physical" }, "missing_information": "none" }

        10. CHECK_PRICE (Checking the price of a coin)
        { "intent": "CHECK_PRICE", "coin": "BTC", "missing_information": "none" }

        11. CHECK_BALANCE (Checking wallet balances)
        { "intent": "CHECK_BALANCE", "missing_information": "none" }

        12. HELP (Asking for commands or help)
        { "intent": "HELP", "missing_information": "none" }

        13. UNKNOWN (Anything else)
        { "intent": "UNKNOWN", "missing_information": "none" }

        CRITICAL RULES:
        - If they want to perform an action but are missing details (like the phone number, address, network, amount, bank name, or coin), set "missing_information" to a polite question asking for it.
        - If they only set a TP, leave stop_loss as null (and vice versa).
        - Return ONLY valid JSON without any markdown formatting.
        `;

        const result = await model.generateContent(prompt);
        let responseText = result.response.text();
        
        // Clean up markdown block formatting if Gemini includes it
        responseText = responseText.replace(/```json/g, '').replace(/```/g, '').trim();

        return JSON.parse(responseText);

    } catch (error) {
        console.error("AI Parser Error:", error);
        return { intent: "ERROR", missing_information: "System error parsing command." };
    }
}