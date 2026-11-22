import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Testes de componente do Console (HR-39a): Vitest + Testing Library em jsdom.
// O CSS da SofaUI é importado pelos componentes; o Vitest o trata como módulo vazio e o teste
// olha só comportamento e acessibilidade (papel, rótulo, teclado).
export default defineConfig({
    plugins: [react()],
    test: {
        environment: "jsdom",
        setupFiles: ["./src/testes/preparo.ts"],
        include: ["src/**/*.test.{ts,tsx}"],
        css: false,
        restoreMocks: true,
    },
    resolve: { dedupe: ["react", "react-dom"] },
});
