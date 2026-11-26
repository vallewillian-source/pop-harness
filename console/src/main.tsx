
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import "@sofacoding/sofa-ui/styles.css";
import "./main.css";
import "./styles/relayout.css";
import { App } from "./App";
import { I18nProvider } from "./i18n";
import { SessionProvider } from "./session/Session";

const root = document.getElementById("root");
if (!root) {
    throw new Error("elemento #root ausente no index.html");
}

createRoot(root).render(
    <StrictMode>
        <BrowserRouter>
            <I18nProvider>
                <SessionProvider>
                    <App />
                </SessionProvider>
            </I18nProvider>
        </BrowserRouter>
    </StrictMode>,
);
