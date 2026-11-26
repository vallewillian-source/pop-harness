import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
    plugins: [react()],
    test: {
        environment: "jsdom",
        setupFiles: ["./src/testing/setup.ts"],
        include: ["src/**/*.test.{ts,tsx}"],
        css: false,
        restoreMocks: true,
    },
    resolve: { dedupe: ["react", "react-dom"] },
});
