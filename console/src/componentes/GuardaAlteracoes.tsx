/**
 * Aviso de alterações não salvas (UX_SPEC §4.1, HR-40).
 *
 * O Console usa `BrowserRouter`, que não tem o `useBlocker` dos data routers. Então a guarda é
 * feita em dois pontos, que é tudo o que a tela de Instruções precisa:
 *
 * 1. **Saída da página:** um ouvinte de `click` em fase de captura intercepta qualquer link interno
 *    (rail, trilha, lista de agentes) quando há edição pendente, e devolve o destino ao diálogo.
 *    Navegação feita por código passa por `tentarNavegar`, que faz a mesma coisa.
 * 2. **Fechar/recarregar:** `beforeunload` enquanto houver edição pendente.
 *
 * A tela registra `{ sujo, salvar }`; o diálogo oferece salvar e sair, descartar ou cancelar. O
 * botão voltar do navegador não passa por aqui — está nos limites conhecidos do relatório.
 */

import {
    createContext,
    useCallback,
    useContext,
    useEffect,
    useMemo,
    useRef,
    useState,
    type ReactNode,
} from "react";
import { useNavigate } from "react-router-dom";
import { SofaButton, SofaModal } from "@sofacoding/sofa-ui";

export type RegistroDaGuarda = {
    /** Há edição pendente na tela atual? */
    sujo: boolean;
    /** Salva; `true` quando pode sair (o `412` devolve `false` e a tela mostra o conflito). */
    salvar: () => Promise<boolean>;
};

type Contexto = {
    /** Registra (ou limpa, com `null`) o estado da tela atual. */
    registrar: (registro: RegistroDaGuarda | null) => void;
    /** Navega se não houver edição pendente; `true` quando a navegação foi feita. */
    tentarNavegar: (para: string) => boolean;
};

const GuardaContexto = createContext<Contexto | null>(null);

/** Estado da tela atual + navegação com guarda. Exige o `GuardaAlteracoesProvider`. */
export function useGuardaDeAlteracoes(): Contexto {
    const contexto = useContext(GuardaContexto);
    if (!contexto) {
        throw new Error("useGuardaDeAlteracoes exige GuardaAlteracoesProvider");
    }
    return contexto;
}

/** Caminho interno do link, ou `null` quando o clique deve seguir como está. */
function destinoDoLink(evento: MouseEvent): string | null {
    // Clique com modificador, botão do meio/direito ou link que abre fora não é navegação interna.
    if (evento.defaultPrevented || evento.button !== 0) return null;
    if (evento.metaKey || evento.ctrlKey || evento.shiftKey || evento.altKey) return null;

    const alvo = evento.target as HTMLElement | null;
    const link = alvo?.closest?.("a[href]") as HTMLAnchorElement | null;
    if (!link || link.target === "_blank" || link.hasAttribute("download")) return null;

    let url: URL;
    try {
        url = new URL(link.href, window.location.origin);
    } catch {
        return null;
    }
    if (url.origin !== window.location.origin) return null;

    const atual = `${window.location.pathname}${window.location.search}`;
    const destino = `${url.pathname}${url.search}`;
    // Mesmo caminho (âncora na própria página) não sai da tela.
    if (destino === atual) return null;
    return `${destino}${url.hash}`;
}

export function GuardaAlteracoesProvider({ children }: { children: ReactNode }) {
    const navegar = useNavigate();
    const registro = useRef<RegistroDaGuarda | null>(null);
    const [destino, setDestino] = useState<string | null>(null);
    const [salvando, setSalvando] = useState(false);

    const registrar = useCallback((novo: RegistroDaGuarda | null) => {
        registro.current = novo;
    }, []);

    const sujo = destino === null ? (registro.current?.sujo ?? false) : true;

    useEffect(() => {
        const aoSair = (evento: BeforeUnloadEvent) => {
            if (!registro.current?.sujo) return;
            evento.preventDefault();
            // Navegadores modernos ignoram o texto e mostram a mensagem própria.
            evento.returnValue = "";
        };
        window.addEventListener("beforeunload", aoSair);
        return () => window.removeEventListener("beforeunload", aoSair);
    }, []);

    useEffect(() => {
        const aoClicar = (evento: MouseEvent) => {
            if (!registro.current?.sujo) return;
            const para = destinoDoLink(evento);
            if (!para) return;
            evento.preventDefault();
            evento.stopPropagation();
            setDestino(para);
        };
        document.addEventListener("click", aoClicar, true);
        return () => document.removeEventListener("click", aoClicar, true);
    }, []);

    const tentarNavegar = useCallback(
        (para: string) => {
            if (registro.current?.sujo) {
                setDestino(para);
                return false;
            }
            navegar(para);
            return true;
        },
        [navegar],
    );

    const valor = useMemo(() => ({ registrar, tentarNavegar }), [registrar, tentarNavegar]);

    async function salvarESair() {
        const alvo = destino;
        if (!alvo || !registro.current) return;
        setSalvando(true);
        try {
            const podeSair = await registro.current.salvar();
            if (!podeSair) {
                // A tela mostra o conflito no formulário e o operador continua onde estava.
                setDestino(null);
                return;
            }
            registro.current = null;
            setDestino(null);
            navegar(alvo);
        } finally {
            setSalvando(false);
        }
    }

    return (
        <GuardaContexto.Provider value={valor}>
            {children}
            <SofaModal
                open={destino !== null}
                onOpenChange={(aberto) => {
                    if (!aberto && !salvando) setDestino(null);
                }}
                title="Alterações não salvas"
                description={
                    sujo
                        ? "Você editou as instruções do agente e ainda não salvou. O que sair da tela agora descarta o rascunho."
                        : "Há uma edição pendente nesta tela."
                }
                closeLabel="Fechar aviso de alterações não salvas"
                size="sm"
                id="poph-alteracoes-nao-salvas"
                footer={
                    <>
                        <SofaButton variant="secondary" disabled={salvando} onClick={() => setDestino(null)}>
                            Cancelar
                        </SofaButton>
                        <SofaButton
                            variant="danger"
                            disabled={salvando}
                            onClick={() => {
                                registro.current = null;
                                const alvo = destino;
                                setDestino(null);
                                if (alvo) navegar(alvo);
                            }}
                        >
                            Descartar alterações
                        </SofaButton>
                        <SofaButton
                            variant="primary"
                            loading={salvando}
                            onClick={() => void salvarESair()}
                        >
                            Salvar e sair
                        </SofaButton>
                    </>
                }
            >
                <p style={{ margin: 0, color: "var(--pop-text-secondary)" }}>
                    Salve para manter o que você digitou, ou descarte para sair sem gravar.
                </p>
            </SofaModal>
        </GuardaContexto.Provider>
    );
}
