
import { waitFor } from "@testing-library/react";

export async function waitForModalFocus(): Promise<void> {
    await waitFor(() => {
        const dialog = document.querySelector('[role="dialog"]');
        if (!dialog) throw new Error("o modal ainda não abriu");
        const ativo = document.activeElement;
        if (!ativo || !dialog.contains(ativo)) {
            throw new Error("o foco inicial do modal ainda não foi aplicado");
        }
    });
}
