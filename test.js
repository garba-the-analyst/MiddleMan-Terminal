const textToTest = "Send 5000 to Palmpay account 8031234567"; // You can change this to test different features!

const payload = {
  "object": "whatsapp_business_account",
  "entry": [{
    "changes": [{
      "value": {
        "messages": [{
          "from": "2348031234567", // Simulated user phone number
          "type": "text",
          "text": { "body": textToTest }
        }]
      }
    }]
  }]
};

// Send the fake WhatsApp message to our local server
fetch('http://localhost:3000/webhook', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload)
})
.then(res => console.log(`Test message sent! Server responded with status: ${res.status}`))
.catch(err => console.error("Failed to connect. Is your server running?", err.message));