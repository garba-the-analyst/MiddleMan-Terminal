/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{vue,js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        // Core Theme Palette
        obsidian: {
          DEFAULT: '#0A0E1A',
          dark: '#050810',
          card: '#111827',
          border: '#1F2937',
        },
        navy: {
          DEFAULT: '#0F1C3F',
          dark: '#0A1128',
          light: '#1E293B',
          accent: '#1D4ED8',
        },
        silver: {
          DEFAULT: '#C0C0C0',
          metallic: '#CBD5E1',
          light: '#E2E8F0',
          muted: '#94A3B8',
          dark: '#64748B',
        },
      },
    },
  },
  plugins: [],
}