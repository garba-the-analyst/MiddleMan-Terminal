import pg from 'pg';
import 'dotenv/config';

const { Pool } = pg;

if (!process.env.NEON_DATABASE_URL) {
    throw new Error("Missing NEON_DATABASE_URL in environment variables.");
}

const pool = new Pool({
    connectionString: process.env.NEON_DATABASE_URL,
    ssl: {
        rejectUnauthorized: false // Required for Neon
    }
});

export default pool;