import { useSyncExternalStore } from "react";

type Entry = { agent: string; model: string | null };

let current: Entry | null = null;
const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => {
        listeners.delete(listener);
    };
}

export function publishAgentModel(agent: string, model: string | null): void {
    const next = model?.trim() ? model.trim() : null;
    if (current?.agent === agent && current.model === next) return;
    current = { agent, model: next };
    listeners.forEach((listener) => listener());
}

export function publishedAgentModel(agent: string): string | null {
    return current?.agent === agent ? current.model : null;
}

export function useAgentModel(agent: string | undefined): string | null {
    useSyncExternalStore(subscribe, () => current);
    return agent ? publishedAgentModel(agent) : null;
}
