/**
 * Espera o modal assentar antes de interagir.
 *
 * O `SofaModal` prende o foco com o `FloatingFocusManager`, que aplica o foco inicial num passo
 * próprio depois do primeiro render. Esperar um tempo fixo (30 ms) funcionava quase sempre, mas sob
 * carga (suíte inteira em paralelo, CPU ocupada) o passo do foco atrasa e a digitação se perde num
 * campo que ainda não estava focado — o teste reprovava sem haver defeito no produto.
 *
 * Aqui a espera é pelo **estado**: o diálogo aparece e o foco está dentro dele. No navegador é
 * invisível; no teste, elimina a corrida.
 */

import { waitFor } from "@testing-library/react";

export async function esperarFocoDoModal(): Promise<void> {
    await waitFor(() => {
        const dialogo = document.querySelector('[role="dialog"]');
        if (!dialogo) throw new Error("o modal ainda não abriu");
        const ativo = document.activeElement;
        if (!ativo || !dialogo.contains(ativo)) {
            throw new Error("o foco inicial do modal ainda não foi aplicado");
        }
    });
}
