import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        canvas: "#0f0f0f",
        panel: "#181818",
        surface: "#212121",
        accent: "#ff0000",
      },
      boxShadow: {
        panel: "0 12px 34px rgba(0, 0, 0, 0.22)",
      },
    },
  },
  plugins: [],
} satisfies Config;
