import fetch from 'node-fetch'; // Make sure you have node-fetch installed, or use native fetch if on Node 18+
import 'dotenv/config';

const FLW_SECRET_KEY = process.env.FLUTTERWAVE_SECRET_KEY;
const BASE_URL = 'https://api.flutterwave.com/v3';

// 1. BUY AIRTIME / DATA
export async function buyAirtime(phone, amount, network) {
    try {
        // Flutterwave bills endpoint mapping (simulated payload)
        const payload = {
            country: 'NG',
            customer: phone,
            amount: amount,
            type: 'AIRTIME', 
            biller_name: network.toUpperCase() // e.g., 'MTN', 'AIRTEL'
        };

        const response = await fetch(`${BASE_URL}/bills`, {
            method: 'POST',
            headers: {
                'Authorization': `Bearer ${FLW_SECRET_KEY}`,
                'Content-Type': 'application/json'
            },
            body: JSON.stringify(payload)
        });

        const data = await response.json();

        // If in Sandbox, we might mock a success response if keys aren't set yet
        if (!FLW_SECRET_KEY) {
            console.log("⚠️ No API Key found. Simulating successful airtime purchase:", payload);
            return { success: true, reference: `MOCK_TX_${Date.now()}` };
        }

        if (data.status === 'success') {
            return { success: true, reference: data.data.reference };
        } else {
            return { success: false, error: data.message };
        }

    } catch (error) {
        console.error('Airtime API Error:', error);
        return { success: false, error: 'Provider connection failed.' };
    }
}

// 2. WITHDRAW TO EXTERNAL BANK
export async function withdrawFiat(accountNumber, bankCode, amount, narration = "MiddleMan Withdrawal") {
    try {
        const payload = {
            account_bank: bankCode, // e.g., '044' for Access Bank, etc.
            account_number: accountNumber,
            amount: amount,
            narration: narration,
            currency: "NGN",
            reference: `MM_WD_${Date.now()}`
        };

        const response = await fetch(`${BASE_URL}/transfers`, {
            method: 'POST',
            headers: {
                'Authorization': `Bearer ${FLW_SECRET_KEY}`,
                'Content-Type': 'application/json'
            },
            body: JSON.stringify(payload)
        });

        const data = await response.json();

        // Mock success if no API key is present for testing
        if (!FLW_SECRET_KEY) {
            console.log("⚠️ No API Key found. Simulating successful bank withdrawal:", payload);
            return { success: true, status: 'PROCESSING', reference: payload.reference };
        }

        if (data.status === 'success') {
            return { success: true, status: data.data.status, reference: data.data.reference };
        } else {
            return { success: false, error: data.message };
        }

    } catch (error) {
        console.error('Fiat Withdrawal Error:', error);
        return { success: false, error: 'Bank network connection failed.' };
    }
}