import { v2 as cloudinary } from 'cloudinary';
import 'dotenv/config';

cloudinary.config({
    cloud_name: process.env.CLOUDINARY_CLOUD_NAME,
    api_key: process.env.CLOUDINARY_API_KEY,
    api_secret: process.env.CLOUDINARY_API_SECRET,
    secure: true
});

export async function processWhatsAppImage(base64Data, mimetype) {
    try {
        if (!process.env.CLOUDINARY_API_KEY) {
            console.log("No Cloudinary keys found. Simulating image upload...");
            return "https://via.placeholder.com/400x200/222222/ffffff?text=Simulated+Upload";
        }

        const dataUri = `data:${mimetype};base64,${base64Data}`;

        const result = await cloudinary.uploader.upload(dataUri, {
            folder: "middleman_giftcards",
            resource_type: "image",
        });

        console.log(`✅ Image uploaded to Cloudinary: ${result.secure_url}`);
        return result.secure_url;

    } catch (error) {
        console.error("Error uploading to Cloudinary:", error);
        throw new Error("Failed to upload image.");
    }
}