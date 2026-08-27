import { v2 as cloudinary } from 'cloudinary';
import fs from 'fs';
import path from 'path';
import { config, cloudinaryEnabled } from '../config';

if (cloudinaryEnabled) {
  cloudinary.config({
    cloud_name: config.CLOUDINARY_CLOUD_NAME,
    api_key: config.CLOUDINARY_API_KEY,
    api_secret: config.CLOUDINARY_API_SECRET,
  });
}

const tempDir = path.join(process.cwd(), 'temp_media');
fs.mkdirSync(tempDir, { recursive: true });

export async function uploadCardImage(buffer: Buffer, messageId: string): Promise<string> {
  if (!cloudinaryEnabled) {
    throw new Error('cloudinary credentials not configured');
  }
  const filePath = path.join(tempDir, `${messageId}.jpg`);
  try {
    await fs.promises.writeFile(filePath, buffer);
    const result = await cloudinary.uploader.upload(filePath, {
      folder: 'middleman_trades',
      public_id: messageId,
      resource_type: 'image',
    });
    return result.secure_url;
  } finally {
    await fs.promises.rm(filePath, { force: true });
  }
}
