import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// O Console é servido pelo próprio binário `poph serve` em `/` (HR-38). Em desenvolvimento o Vite
// faz o proxy da API para o servidor local, para a origem ser a mesma do build embutido.
export default defineConfig({
    plugins: [react()],
    build: {
        outDir: "dist",
        // O binário embute os arquivos e serve a raiz `/`; caminho absoluto evita surpresa em
        // rota profunda (`/orgs/<id>`).
        assetsDir: "assets",
        sourcemap: false,
    },
    // A SofaUI declara react/react-dom como peer: uma cópia só em runtime.
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
