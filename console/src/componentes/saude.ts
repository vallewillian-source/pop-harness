/**
 * Saúde da instância, compartilhada pela casca (rail e barra superior).
 *
 * Fica num módulo próprio porque os dois componentes consultam a saúde de forma independente:
 * cada um mantém o próprio laço de 15 s, como antes da separação do shell.
 *
 * `degradada` é o caso em que `/healthz` responde 503 (um banco não responde). Ele não é "sem
 * conexão": a instância respondeu, só que com problema. `semConexao` é quando nem a resposta chega.
 */

import { useEffect, useState } from "react";
import { ErroApi, saude, type Saude } from "../api/cliente";
import { consultarMetricas } from "../api/operacao";

const INTERVALO_SAUDE_MS = 15_000;

export type SituacaoDaSaude = {
    saude: Saude | null;
    semConexao: boolean;
    degradada: boolean;
};

/** Saúde da instância, consultada sem token enquanto o componente está montado. */
export function useSaude(): SituacaoDaSaude {
    const [dados, setDados] = useState<Saude | null>(null);
    const [semConexao, setSemConexao] = useState(false);
    const [degradada, setDegradada] = useState(false);

    useEffect(() => {
        let vivo = true;
        const consultar = () => {
            saude()
                .then((resposta) => {
                    if (!vivo) return;
                    setDados(resposta);
                    setSemConexao(false);
                    setDegradada(resposta.status !== "ok");
                })
                .catch((erro: unknown) => {
                    if (!vivo) return;
                    const respondeuComErro = erro instanceof ErroApi && erro.status === 503;
                    setSemConexao(!respondeuComErro);
                    setDegradada(respondeuComErro);
                });
        };
        consultar();
        const id = window.setInterval(consultar, INTERVALO_SAUDE_MS);
        return () => {
            vivo = false;
            window.clearInterval(id);
        };
    }, []);

    return { saude: dados, semConexao, degradada };
}

/**
 * Memória residente do processo, em bytes, lida de `/metrics` (sem token). `null` enquanto não vem
 * ou quando a consulta falha: a tela então não mostra o número.
 */
export function useMemoriaDoProcesso(): number | null {
    const [bytes, setBytes] = useState<number | null>(null);

    useEffect(() => {
        let vivo = true;
        const consultar = () => {
            consultarMetricas()
                .then((medicoes) => {
                    if (!vivo) return;
                    setBytes(medicoes.memoriaProcesso ?? null);
                })
                .catch(() => {
                    if (!vivo) return;
                    setBytes(null);
                });
        };
        consultar();
        const id = window.setInterval(consultar, INTERVALO_SAUDE_MS);
        return () => {
            vivo = false;
            window.clearInterval(id);
        };
    }, []);

    return bytes;
}
