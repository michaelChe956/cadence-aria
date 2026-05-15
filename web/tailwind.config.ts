import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: "#17202A",
        panel: "#FFFFFF",
        panelMuted: "#FBFCFE",
        line: "#DDE5EC",
        signal: "#0891B2",
        caution: "#D97706",
        danger: "#DC2626",
        success: "#059669",
      },
    },
  },
  plugins: [],
} satisfies Config;
