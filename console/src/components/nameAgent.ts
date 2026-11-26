
import { useSyncExternalStore } from "react";

type Published = { agente: string; nome: string };

let current: Published | null = null;
const listeners = new Set<() => void>();

function inscrever(listener: () => void): () => void {
    listeners.add(listener);
    return () => {
        listeners.delete(listener);
    };
}

export function publishNameAgent(agente: string, nome: string): void {
    if (current?.agente === agente && current.nome === nome) return;
    current = { agente, nome };
    listeners.forEach((listener) => listener());
}

export function nameAgentPublished(agente: string): string | null {
    return current?.agente === agente ? current.nome : null;
}

export function useNameAgent(agente: string | undefined): string | null {
    useSyncExternalStore(inscrever, () => current);
    return agente ? nameAgentPublished(agente) : null;
}
