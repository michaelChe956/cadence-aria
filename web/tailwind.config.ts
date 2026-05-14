import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: "#101418",
        panel: "#f6f8f9",
        line: "#d8e0e6",
        signal: "#14b8a6",
        caution: "#f59e0b",
        danger: "#dc2626",
      },
    },
  },
  plugins: [],
} satisfies Config;
