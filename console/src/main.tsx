/**
 * Entrada do Console. A ordem de importação de CSS importa: os estilos da SofaUI vêm primeiro
 * (tokens e componentes) e o CSS do app depois (fundação do `body` e classes da casca).
 */

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import "@sofacoding/sofa-ui/styles.css";
import "./main.css";
/* Base do re-layout do Console (RL-01): blocos reutilizáveis com prefixo `poph-`. */
import "./estilos/relayout.css";
import { App } from "./App";
import { SessaoProvider } from "./sessao/Sessao";

const raiz = document.getElementById("root");
if (!raiz) {
    throw new Error("elemento #root ausente no index.html");
}

createRoot(raiz).render(
    <StrictMode>
        <BrowserRouter>
            <SessaoProvider>
                <App />
            </SessaoProvider>
        </BrowserRouter>
    </StrictMode>,
);
