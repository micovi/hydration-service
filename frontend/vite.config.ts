import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";
import tailwindcss from "@tailwindcss/vite";

// https://vite.dev/config/
export default defineConfig({
  base: "/",
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    allowedHosts: ["hb.zoao.dev", "hb-hs.zoao.dev", "localhost"],
    proxy: {
      "/hydration-service/hb-node": {
        target: "http://65.108.7.125:8734",
        changeOrigin: true,
        rewrite: (p) => p.replace(/^\/hydration-service\/hb-node/, ""),
      },
      "/hydration-service/backend": {
        target: "http://backend:8081",
        changeOrigin: true,
        rewrite: (p) => p.replace(/^\/hydration-service\/backend/, ""),
      },
    },
  },
});
