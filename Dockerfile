# Use the official Puppeteer image (comes with Chrome pre-installed!)
FROM ghcr.io/puppeteer/puppeteer:latest

# Tell Puppeteer to use the pre-installed Chrome
ENV PUPPETEER_SKIP_CHROMIUM_DOWNLOAD=true \
    PUPPETEER_EXECUTABLE_PATH=/usr/bin/google-chrome-stable

# Create app directory
WORKDIR /usr/src/app

# Copy package files (Switch to the root user temporarily to avoid permission errors)
USER root
COPY package*.json ./
RUN npm install

# Copy the rest of the application code
COPY . .

# Hugging Face REQUIRES your web server to listen on port 7860
EXPOSE 7860

# Start the bot
CMD ["node", "server.js"]