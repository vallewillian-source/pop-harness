import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
    plugins: [react()],
    build: {
        outDir: "dist",
        assetsDir: "assets",
        sourcemap: false,
    },
    resolve: { dedupe: ["react", "react-dom"] },
    server: {
        port: 5199,
        strictPort: true,
        proxy: {
            "/admin": "http://127.0.0.1:7480",
            "/healthz": "http://127.0.0.1:7480",
            "/metrics": "http://127.0.0.1:7480",
        },
    },
});
