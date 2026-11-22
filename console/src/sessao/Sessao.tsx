/**
 * Sessão do Console: token administrativo, escopo e a lista de orgs que o token enxerga.
 *
 * O token vive **na memória da aba e no `sessionStorage`** (UX_SPEC §3.1): fechar a aba encerra a
 * sessão; nada vai para `localStorage`. A tela de login avisa isso.
 */

import {
    createContext,
    useCallback,
    useContext,
    useEffect,
    useMemo,
    useState,
    type ReactNode,
} from "react";
import {
    ErroApi,
    definirToken,
    pedir,
    quandoExpirar,
    type ListaOrgs,
    type Org,
} from "../api/cliente";

const CHAVE = "poph-console.sessao";

/** Alcance do token: global (todas as orgs) ou de uma org só. */
export type Escopo = "global" | "org";

/** Sessão aberta. */
export type Sessao = {
    token: string;
    escopo: Escopo;
    orgs: Org[];
};

type Contexto = {
    sessao: Sessao | null;
    /** `true` quando a última sessão caiu por token recusado (mostra o aviso no login). */
    expirada: boolean;
    entrar: (token: string) => Promise<Sessao>;
    sair: () => void;
    recarregarOrgs: () => Promise<void>;
    limparAvisoExpiracao: () => void;
};

const SessaoContexto = createContext<Contexto | null>(null);

/**
 * O prefixo do token diz o escopo (PRD §8.1): `pophg_` é global, `poph_` é de org. Quando o valor
 * não segue nenhum dos dois, o escopo vem da própria resposta — uma org só significa token de org.
 */
function escopoDoToken(token: string, orgs: Org[]): Escopo {
    if (token.startsWith("pophg_")) return "global";
    if (token.startsWith("poph_")) return "org";
    return orgs.length > 1 ? "global" : "org";
}

function lerDaAba(): Sessao | null {
    try {
        const texto = window.sessionStorage.getItem(CHAVE);
        if (!texto) return null;
        const lido = JSON.parse(texto) as Partial<Sessao>;
        if (typeof lido.token !== "string" || !Array.isArray(lido.orgs)) return null;
        return {
            token: lido.token,
            escopo: lido.escopo === "org" ? "org" : "global",
            orgs: lido.orgs as Org[],
        };
    } catch {
        return null;
    }
}

function gravarNaAba(sessao: Sessao | null): void {
    try {
        if (sessao) {
            window.sessionStorage.setItem(CHAVE, JSON.stringify(sessao));
        } else {
            window.sessionStorage.removeItem(CHAVE);
        }
    } catch {
        // Aba com armazenamento bloqueado: a sessão continua só em memória.
    }
}

export function SessaoProvider({ children }: { children: ReactNode }) {
    const [sessao, setSessao] = useState<Sessao | null>(() => {
        const inicial = lerDaAba();
        definirToken(inicial?.token ?? null);
        return inicial;
    });
    const [expirada, setExpirada] = useState(false);

    const sair = useCallback(() => {
        definirToken(null);
        gravarNaAba(null);
        setSessao(null);
    }, []);

    useEffect(() => {
        quandoExpirar(() => {
            setExpirada(true);
            sair();
        });
        return () => quandoExpirar(null);
    }, [sair]);

    const entrar = useCallback(async (token: string) => {
        const limpo = token.trim();
        if (!limpo) {
            throw new ErroApi(400, "token_vazio", "informe o token administrativo");
        }
        definirToken(limpo);
        try {
            // Validação de token: o 401 é erro de digitação, não fim de sessão.
            const lista = await pedir<ListaOrgs>("/admin/v1/orgs?limit=200", {}, { encerraSessao: false });
            const nova: Sessao = {
                token: limpo,
                escopo: escopoDoToken(limpo, lista.items),
                orgs: lista.items,
            };
            gravarNaAba(nova);
            setSessao(nova);
            setExpirada(false);
            return nova;
        } catch (erro) {
            definirToken(null);
            throw erro;
        }
    }, []);

    const recarregarOrgs = useCallback(async () => {
        const lista = await pedir<ListaOrgs>("/admin/v1/orgs?limit=200");
        setSessao((atual) => {
            if (!atual) return atual;
            const nova: Sessao = { ...atual, orgs: lista.items };
            gravarNaAba(nova);
            return nova;
        });
    }, []);

    const limparAvisoExpiracao = useCallback(() => setExpirada(false), []);

    const valor = useMemo<Contexto>(
        () => ({ sessao, expirada, entrar, sair, recarregarOrgs, limparAvisoExpiracao }),
        [sessao, expirada, entrar, sair, recarregarOrgs, limparAvisoExpiracao],
    );

    return <SessaoContexto.Provider value={valor}>{children}</SessaoContexto.Provider>;
}

/** Sessão e ações. Só pode ser usado dentro de `SessaoProvider`. */
export function useSessao(): Contexto {
    const contexto = useContext(SessaoContexto);
    if (!contexto) {
        throw new Error("useSessao precisa estar dentro de <SessaoProvider>");
    }
    return contexto;
}
