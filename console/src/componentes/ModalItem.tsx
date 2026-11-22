/**
 * Formulário de item (trecho) da base, em modal (HR-41).
 *
 * Título até 200 caracteres e conteúdo até 1000 — o mesmo limite do servidor
 * (`LIMITE_CONTEUDO_ITEM`), mostrado como contador ao vivo. Concorrência otimista igual às outras
 * telas: `PATCH` com `If-Match`; no `412` a tela relê o item, adota a versão atual e mantém o texto
 * digitado.
 *
 * A resposta de `POST`/`PATCH` traz `index_warning` quando o índice não pôde ser sincronizado; a
 * tela repassa o aviso para quem chamou (a aba mostra o estado e oferece reindexar).
 */

import { useEffect, useState, type FormEvent } from "react";
import {
    SofaBanner,
    SofaButton,
    SofaField,
    SofaInput,
    SofaModal,
    SofaSelect,
    SofaTextarea,
    type SofaFieldControlProps,
} from "@sofacoding/sofa-ui";
import { ErroApi } from "../api/cliente";
import {
    atualizarItem,
    criarItem,
    escreverTags,
    lerTags,
    LIMITE_CONTEUDO_ITEM,
    LIMITE_TITULO_ITEM,
    listarItens,
    type ItemConhecimento,
} from "../api/conhecimento";

const SITUACOES = [
    { valor: "active", rotulo: "Ativo" },
    { valor: "inactive", rotulo: "Inativo" },
    { valor: "archived", rotulo: "Arquivado" },
];

export type ModalItemProps = {
    org: string;
    grupo: string;
    aberto: boolean;
    aoFechar: () => void;
    /** Item em edição; `null` cria um novo. */
    item: ItemConhecimento | null;
    /** Chamado com o item devolvido pela API (criado ou alterado). */
    aoSalvar: (item: ItemConhecimento) => void;
};

type Erros = { titulo?: string; conteudo?: string; ordem?: string };

/** Nome do campo da API → campo do formulário. */
const CAMPOS_DA_API: Record<string, keyof Erros> = {
    title: "titulo",
    titulo: "titulo",
    content: "conteudo",
    conteudo: "conteudo",
};

/** Ligações do `SofaField` sem `describedBy`, que vira `aria-describedby` no controle. */
function semDescribedBy(controle: SofaFieldControlProps) {
    const { describedBy, ...resto } = controle;
    return { ...resto, "aria-describedby": describedBy };
}

/** Releitura usada no `412`: o item vivo (pode ter mais de uma página por grupo). */
async function relerItem(
    org: string,
    grupo: string,
    id: string,
): Promise<ItemConhecimento | null> {
    const itens = await listarItens(org, grupo);
    return itens.find((candidato) => candidato.id === id) ?? null;
}

export function ModalItem({ org, grupo, aberto, aoFechar, item, aoSalvar }: ModalItemProps) {
    const editando = item !== null;
    const [titulo, setTitulo] = useState("");
    const [conteudo, setConteudo] = useState("");
    const [tags, setTags] = useState("");
    const [posicao, setPosicao] = useState("0");
    const [situacao, setSituacao] = useState("active");
    const [versao, setVersao] = useState(0);
    const [erros, setErros] = useState<Erros>({});
    const [erroGeral, setErroGeral] = useState<string | null>(null);
    const [conflito, setConflito] = useState<string | null>(null);
    const [enviando, setEnviando] = useState(false);

    useEffect(() => {
        if (!aberto) return;
        setTitulo(item?.title ?? "");
        setConteudo(item?.content ?? "");
        setTags(escreverTags(item?.tags ?? []));
        setPosicao(String(item?.position ?? 0));
        setSituacao(item?.status ?? "active");
        setVersao(item?.version ?? 0);
        setErros({});
        setErroGeral(null);
        setConflito(null);
        setEnviando(false);
    }, [aberto, item]);

    const caracteres = conteudo.trim().length;

    function validar(): Erros {
        const novos: Erros = {};
        if (!titulo.trim()) {
            novos.titulo = "informe o título do trecho";
        } else if (titulo.trim().length > LIMITE_TITULO_ITEM) {
            novos.titulo = `o título aceita até ${LIMITE_TITULO_ITEM} caracteres`;
        }
        if (!conteudo.trim()) {
            novos.conteudo = "informe o conteúdo do trecho";
        } else if (caracteres > LIMITE_CONTEUDO_ITEM) {
            novos.conteudo = `o conteúdo aceita até ${LIMITE_CONTEUDO_ITEM} caracteres; este tem ${caracteres}`;
        }
        if (posicao.trim() && !Number.isInteger(Number(posicao))) {
            novos.ordem = "a ordem precisa ser um número inteiro";
        }
        return novos;
    }

    /** Põe o erro da API no campo que ele aponta; o que não tem campo vai para o aviso geral. */
    function mostrarErroDaApi(falha: ErroApi) {
        const detalhes = falha.detalhes as Record<string, unknown> | null | undefined;
        const campo = typeof detalhes?.["field"] === "string" ? detalhes["field"] : undefined;
        const motivo =
            typeof detalhes?.["reason"] === "string" ? detalhes["reason"] : falha.message;
        const alvo = campo ? CAMPOS_DA_API[campo] : undefined;
        if (alvo) {
            setErros({ [alvo]: motivo });
            return;
        }
        setErroGeral(`${falha.message} (código ${falha.codigo})`);
    }

    async function salvar(evento?: FormEvent) {
        evento?.preventDefault();
        const novos = validar();
        setErros(novos);
        setErroGeral(null);
        setConflito(null);
        if (Object.keys(novos).length > 0) return;

        const ordem = posicao.trim() ? Number(posicao) : 0;
        setEnviando(true);
        try {
            if (!item) {
                const criado = await criarItem(org, grupo, {
                    title: titulo.trim(),
                    content: conteudo,
                    tags: lerTags(tags),
                    position: ordem,
                });
                aoSalvar(criado);
                aoFechar();
                return;
            }

            const alterado = await atualizarItem(org, grupo, item.id, versao, {
                title: titulo.trim(),
                content: conteudo,
                tags: lerTags(tags),
                position: ordem,
                status: situacao,
            });
            aoSalvar(alterado);
            aoFechar();
        } catch (falha) {
            if (falha instanceof ErroApi && falha.status === 412 && item) {
                let versaoAtual: number | undefined;
                try {
                    const atual = await relerItem(org, grupo, item.id);
                    versaoAtual = atual?.version;
                } catch {
                    // Sem a releitura, a próxima tentativa devolve 412 de novo; nada se perde.
                }
                if (typeof versaoAtual === "number") setVersao(versaoAtual);
                setConflito(
                    `Outra pessoa salvou este trecho antes de você${
                        typeof versaoAtual === "number" ? ` (agora na versão ${versaoAtual})` : ""
                    }. O que você digitou foi mantido: revise e salve de novo.`,
                );
                return;
            }
            if (falha instanceof ErroApi) {
                mostrarErroDaApi(falha);
                return;
            }
            setErroGeral("falha inesperada ao salvar o trecho");
        } finally {
            setEnviando(false);
        }
    }

    return (
        <SofaModal
            open={aberto}
            onOpenChange={(novo) => {
                if (!novo && !enviando) aoFechar();
            }}
            title={editando ? "Editar trecho" : "Novo trecho"}
            description={
                editando
                    ? "Alterando o conteúdo que o agente consulta. O salvamento usa a versão lida."
                    : "Escreva o conteúdo como ele deve ser lido pelo agente: uma resposta completa, sem depender do resto da página."
            }
            closeLabel="Fechar formulário de trecho"
            size="lg"
            id="poph-formulario-item"
            footer={
                <>
                    <SofaButton variant="secondary" disabled={enviando} onClick={aoFechar}>
                        Cancelar
                    </SofaButton>
                    <SofaButton
                        variant="primary"
                        type="button"
                        onClick={() => void salvar()}
                        loading={enviando}
                    >
                        {editando ? "Salvar alterações" : "Criar trecho"}
                    </SofaButton>
                </>
            }
        >
            <form
                id="poph-form-item"
                className="poph-pilha"
                style={{ gap: "var(--pop-spacing-lg)" }}
                onSubmit={(evento) => void salvar(evento)}
                noValidate
            >
                {conflito && (
                    <SofaBanner tone="warn" title="Versão desatualizada">
                        {conflito}
                    </SofaBanner>
                )}
                {erroGeral && (
                    <SofaBanner tone="danger" title="Não foi possível salvar">
                        {erroGeral}
                    </SofaBanner>
                )}

                <SofaField
                    label="Título"
                    hint={`Como o trecho aparece na lista. Até ${LIMITE_TITULO_ITEM} caracteres.`}
                    error={erros.titulo}
                    required
                    htmlFor="poph-item-titulo"
                >
                    {(controle) => (
                        <SofaInput
                            {...semDescribedBy(controle)}
                            value={titulo}
                            autoComplete="off"
                            onChange={(evento) => setTitulo(evento.target.value)}
                        />
                    )}
                </SofaField>

                <SofaField
                    label="Conteúdo"
                    hint={`Até ${LIMITE_CONTEUDO_ITEM} caracteres. É o texto que o agente recebe na consulta.`}
                    error={erros.conteudo}
                    required
                    htmlFor="poph-item-conteudo"
                >
                    {(controle) => (
                        <SofaTextarea
                            {...semDescribedBy(controle)}
                            value={conteudo}
                            rows={8}
                            onChange={(evento) => setConteudo(evento.target.value)}
                        />
                    )}
                </SofaField>

                <div
                    className="poph-mono-mudo"
                    aria-live="polite"
                    style={{
                        marginTop: -8,
                        color:
                            caracteres > LIMITE_CONTEUDO_ITEM
                                ? "var(--pop-danger, #e5484d)"
                                : undefined,
                    }}
                >
                    {caracteres} / {LIMITE_CONTEUDO_ITEM} caracteres
                </div>

                <div className="poph-duas-colunas">
                    <SofaField
                        label="Tags"
                        hint="Opcional. Separe por vírgula; ajudam a busca híbrida."
                        htmlFor="poph-item-tags"
                    >
                        {(controle) => (
                            <SofaInput
                                {...semDescribedBy(controle)}
                                value={tags}
                                autoComplete="off"
                                onChange={(evento) => setTags(evento.target.value)}
                            />
                        )}
                    </SofaField>
                    <SofaField
                        label="Ordem"
                        hint="Número inteiro; menor aparece antes."
                        error={erros.ordem}
                        htmlFor="poph-item-ordem"
                    >
                        {(controle) => (
                            <SofaInput
                                {...semDescribedBy(controle)}
                                value={posicao}
                                inputMode="numeric"
                                onChange={(evento) => setPosicao(evento.target.value)}
                            />
                        )}
                    </SofaField>
                </div>

                {editando && (
                    <SofaField
                        label="Situação"
                        hint="Trecho inativo sai da busca depois da próxima sincronização; o texto continua no banco."
                        htmlFor="poph-item-situacao"
                    >
                        {(controle) => (
                            <SofaSelect
                                {...semDescribedBy(controle)}
                                value={situacao}
                                onChange={(evento) => setSituacao(evento.target.value)}
                            >
                                {SITUACOES.map((opcao) => (
                                    <option key={opcao.valor} value={opcao.valor}>
                                        {opcao.rotulo}
                                    </option>
                                ))}
                            </SofaSelect>
                        )}
                    </SofaField>
                )}
            </form>
        </SofaModal>
    );
}
