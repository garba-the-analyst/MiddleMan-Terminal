import { z } from 'zod';

const Env = z.object({
  REDIS_URL: z.string().min(1),
  PORT: z.coerce.number().default(3001),
  INTERNAL_API_SECRET: z.string().min(32),
  CLOUDINARY_CLOUD_NAME: z.string().optional(),
  CLOUDINARY_API_KEY: z.string().optional(),
  CLOUDINARY_API_SECRET: z.string().optional(),
  LOG_LEVEL: z.string().default('silent'),
});

export const config = Env.parse(process.env);

export const cloudinaryEnabled =
  !!config.CLOUDINARY_CLOUD_NAME &&
  !!config.CLOUDINARY_API_KEY &&
  !!config.CLOUDINARY_API_SECRET;
