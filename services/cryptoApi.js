export async function getCryptoPrice(query) {
    try {
        const cleanQuery = query.trim();
        
        // 1. REGEX PATTERNS: Detect if the user pasted a contract address
        const isEVM = /^0x[a-fA-F0-9]{40}$/.test(cleanQuery); // Detects ETH, BSC, Polygon, Base etc.
        const isSolana = /^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(cleanQuery); // Detects Solana Base58 addresses
        const isContract = isEVM || isSolana;

        let dexData;

        // 2. FETCH DATA: Route to the correct DexScreener Endpoint
        if (isContract) {
            // If it's a contract, fetch that exact token (100% accurate, no scams)
            const response = await fetch(`https://api.dexscreener.com/latest/dex/tokens/${cleanQuery}`);
            dexData = await response.json();
        } else {
            // If it's a ticker (like WIF or SOL), search for the highest volume pair
            const response = await fetch(`https://api.dexscreener.com/latest/dex/search?q=${cleanQuery}`);
            dexData = await response.json();
        }

        if (dexData.pairs && dexData.pairs.length > 0) {
            // Sort by USD Liquidity to filter out dead/fake meme coins
            const sortedPairs = dexData.pairs.sort((a, b) => (b.liquidity?.usd || 0) - (a.liquidity?.usd || 0));
            const bestPair = sortedPairs[0];

            return {
                found: true,
                name: bestPair.baseToken.name,
                symbol: bestPair.baseToken.symbol.toUpperCase(),
                priceUsd: parseFloat(bestPair.priceUsd).toFixed(6), // 6 decimals is better for cheap meme coins
                chain: bestPair.chainId.toUpperCase(),
                dex: bestPair.dexId.toUpperCase(),
                volume24h: bestPair.volume?.h24 || 0,
                change24h: bestPair.priceChange?.h24 || 0,
                contract: bestPair.baseToken.address,
                url: bestPair.url // This URL triggers the WhatsApp Chart Image Preview!
            };
        }

        // 3. FALLBACK: If it's not on DEXs, check Binance (CEX) for majors like BTC
        if (!isContract) {
            const binanceSymbol = cleanQuery.toUpperCase() + 'USDT';
            const binanceResponse = await fetch(`https://api.binance.com/api/v3/ticker/24hr?symbol=${binanceSymbol}`);
            
            if (binanceResponse.ok) {
                const binanceData = await binanceResponse.json();
                return {
                    found: true,
                    name: cleanQuery.toUpperCase(),
                    symbol: cleanQuery.toUpperCase(),
                    priceUsd: parseFloat(binanceData.lastPrice).toFixed(4),
                    chain: 'CEX',
                    dex: 'BINANCE',
                    volume24h: parseFloat(binanceData.quoteVolume).toFixed(2),
                    change24h: parseFloat(binanceData.priceChangePercent).toFixed(2),
                    contract: 'N/A (Centralized)',
                    url: `https://www.binance.com/en/trade/${cleanQuery.toUpperCase()}_USDT`
                };
            }
        }

        return { found: false };

    } catch (error) {
        console.error("Crypto API Routing Error:", error);
        return { found: false };
    }
}