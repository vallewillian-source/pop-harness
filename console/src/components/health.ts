
import { useEffect, useState } from "react";
import { ApiError, health, type Health } from "../api/client";
import { fetchMetrics } from "../api/operation";

const INTERVAL_HEALTH_MS = 15_000;

export type SituationHealth = {
    health: Health | null;
    disconnected: boolean;
    degraded: boolean;
};

export function useHealth(): SituationHealth {
    const [dados, setData] = useState<Health | null>(null);
    const [withoutConnection, setConnection] = useState(false);
    const [degraded, setDegraded] = useState(false);

    useEffect(() => {
        let live = true;
        const query = () => {
            health()
                .then((response) => {
                    if (!live) return;
                    setData(response);
                    setConnection(false);
                    setDegraded(response.status !== "ok");
                })
                .catch((error: unknown) => {
                    if (!live) return;
                    const respondedError = error instanceof ApiError && error.status === 503;
                    setConnection(!respondedError);
                    setDegraded(respondedError);
                });
        };
        query();
        const id = window.setInterval(query, INTERVAL_HEALTH_MS);
        return () => {
            live = false;
            window.clearInterval(id);
        };
    }, []);

    return { health: dados, disconnected: withoutConnection, degraded };
}

export function useMemoryProcess(): number | null {
    const [bytes, setBytes] = useState<number | null>(null);

    useEffect(() => {
        let live = true;
        const query = () => {
            fetchMetrics()
                .then((measurements) => {
                    if (!live) return;
                    setBytes(measurements.processMemory ?? null);
                })
                .catch(() => {
                    if (!live) return;
                    setBytes(null);
                });
        };
        query();
        const id = window.setInterval(query, INTERVAL_HEALTH_MS);
        return () => {
            live = false;
            window.clearInterval(id);
        };
    }, []);

    return bytes;
}
