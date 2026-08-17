import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      borderWidth: {
        3: "3px",
      },
      colors: {
        ink: "#17202A",
        panel: "#FFFFFF",
        panelMuted: "#FBFCFE",
        line: "#DDE5EC",
        borderStrong: "#3F3F46",
        primary: "#4F46E5",
        primarySoft: "#E0E7FF",
        cta: "#F97316",
        ctaSoft: "#FFEDD5",
        signal: "#4F46E5",
        caution: "#D97706",
        danger: "#DC2626",
        success: "#059669",
      },
    },
  },
  plugins: [],
} satisfies Config;
