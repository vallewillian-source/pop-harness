/**
 * Preparo dos testes de componente.
 *
 * `@testing-library/react` só limpa o DOM entre testes sozinho quando os globais do Vitest estão
 * ligados; aqui os globais ficam desligados de propósito, então a limpeza é explícita.
 */

import { afterEach } from "vitest";
import { cleanup, configure } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";

/**
 * Teto de espera das buscas assíncronas (`findBy*`, `waitFor`) em 5 s, contra o padrão de 1 s.
 *
 * As telas encadeiam buscas (agente → organização → grupos → detalhe do grupo → trechos); com a
 * suíte inteira em paralelo e a máquina ocupada, 1 s estoura às vezes e o teste reprova sem haver
 * defeito no produto. O teto maior só custa tempo quando algo realmente não aparece.
 */
configure({ asyncUtilTimeout: 5_000 });

/**
 * `scrollIntoView` não existe no jsdom e o `SofaTabSwitcher` usa para manter a aba ativa visível.
 * No navegador é rolagem; no teste, é só um no-op.
 */
if (typeof Element !== "undefined" && !Element.prototype.scrollIntoView) {
    Element.prototype.scrollIntoView = function scrollIntoView(): void {};
}

afterEach(() => {
    cleanup();
});
