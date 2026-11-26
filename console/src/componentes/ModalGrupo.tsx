/**
 * Formulário de grupo da base de conhecimento (criar e editar), em modal (HR-41).
 *
 * Mesmo molde do `ModalEstrutura`: criar faz `POST`; editar faz `PATCH` com `If-Match` da versão
 * lida. Um `412` significa que outra pessoa salvou primeiro: a tela relê o grupo, passa a usar a
 * versão atual e **mantém o que foi digitado** para o operador revisar.
 *
 * Nome é obrigatório (até 120) e único na org; a API aponta o campo repetido em `details.field`
 * (`name`/`nome`) e a mensagem volta no próprio campo.
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
import { ApiError } from "../api/client";
import {
    updateGroup,
    createGroup,
    getGroup,
    LIMIT_GROUP_NAME,
    type KnowledgeGroup,
} from "../api/knowledge";
import { useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";

const SITUACOES: { valor: string; labelKey: TranslationKey }[] = [
    { valor: "active", labelKey: "knowledge.statusActive" },
    { valor: "inactive", labelKey: "knowledge.statusInactive" },
    { valor: "archived", labelKey: "knowledge.statusArchived" },
];

export type ModalGrupoProps = {
    org: string;
    aberto: boolean;
    aoFechar: () => void;
    /** Grupo em edição; `null` cria um novo. */
    grupo: KnowledgeGroup | null;
    /** Chamado com o grupo devolvido pela API (criado ou alterado). */
    aoSalvar: (grupo: KnowledgeGroup) => void;
};

type Erros = { nome?: string; descricao?: string };

/** Nome do campo da API → campo do formulário. */
const CAMPOS_DA_API: Record<string, keyof Erros> = {
    name: "nome",
    nome: "nome",
    description: "descricao",
    descricao: "descricao",
};

/** Ligações do `SofaField` sem `describedBy`, que vira `aria-describedby` no controle. */
function semDescribedBy(controle: SofaFieldControlProps) {
    const { describedBy, ...resto } = controle;
    return { ...resto, "aria-describedby": describedBy };
}

export function ModalGrupo({ org, aberto, aoFechar, grupo, aoSalvar }: ModalGrupoProps) {
    const { translate: t } = useI18n();
    const editando = grupo !== null;
    const [nome, setNome] = useState("");
    const [descricao, setDescricao] = useState("");
    const [situacao, setSituacao] = useState("active");
    const [versao, setVersao] = useState(0);
    const [erros, setErros] = useState<Erros>({});
    const [erroGeral, setErroGeral] = useState<string | null>(null);
    const [conflito, setConflito] = useState<string | null>(null);
    const [enviando, setEnviando] = useState(false);

    useEffect(() => {
        if (!aberto) return;
        setNome(grupo?.name ?? "");
        setDescricao(grupo?.description ?? "");
        setSituacao(grupo?.status ?? "active");
        setVersao(grupo?.version ?? 0);
        setErros({});
        setErroGeral(null);
        setConflito(null);
        setEnviando(false);
    }, [aberto, grupo]);

    function validar(): Erros {
        const novos: Erros = {};
        if (!nome.trim()) {
            novos.nome = t("knowledge.groupNameRequired");
        } else if (nome.trim().length > LIMIT_GROUP_NAME) {
            novos.nome = t("knowledge.groupNameTooLong", { limit: LIMIT_GROUP_NAME });
        }
        return novos;
    }

    /** Põe o erro da API no campo que ele aponta; o que não tem campo vai para o aviso geral. */
    function mostrarErroDaApi(falha: ApiError) {
        const detalhes = falha.detalhes as Record<string, unknown> | null | undefined;
        const campo = typeof detalhes?.["field"] === "string" ? detalhes["field"] : undefined;
        const motivo =
            typeof detalhes?.["reason"] === "string" ? detalhes["reason"] : falha.message;
        const alvo = campo ? CAMPOS_DA_API[campo] : undefined;
        if (alvo) {
            setErros({ [alvo]: motivo });
            return;
        }
        setErroGeral(t("knowledge.apiErrorWithCode", {
            message: falha.message,
            code: falha.codigo,
        }));
    }

    async function salvar(evento?: FormEvent) {
        evento?.preventDefault();
        const novos = validar();
        setErros(novos);
        setErroGeral(null);
        setConflito(null);
        if (Object.keys(novos).length > 0) return;

        setEnviando(true);
        try {
            if (!grupo) {
                const criado = await createGroup(org, {
                    name: nome.trim(),
                    description: descricao,
                });
                aoSalvar(criado);
                aoFechar();
                return;
            }

            const alterado = await updateGroup(org, grupo.id, versao, {
                name: nome.trim(),
                description: descricao,
                status: situacao,
            });
            aoSalvar(alterado);
            aoFechar();
        } catch (falha) {
            if (falha instanceof ApiError && falha.status === 412 && grupo) {
                // Relê para continuar de uma versão válida; o que foi digitado não é tocado.
                let versaoAtual: number | undefined;
                try {
                    const atual = await getGroup(org, grupo.id);
                    versaoAtual = atual.version;
                } catch {
                    // Sem a releitura, a próxima tentativa devolve 412 de novo; nada se perde.
                }
                if (typeof versaoAtual === "number") setVersao(versaoAtual);
                setConflito(
                    typeof versaoAtual === "number"
                        ? t("knowledge.groupConflictWithVersion", { version: versaoAtual })
                        : t("knowledge.groupConflict"),
                );
                return;
            }
            if (falha instanceof ApiError) {
                mostrarErroDaApi(falha);
                return;
            }
            setErroGeral(t("knowledge.groupSaveUnexpected"));
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
            title={editando ? t("knowledge.groupEditTitle") : t("knowledge.groupCreateTitle")}
            description={
                editando
                    ? t("knowledge.groupEditDescription", { name: grupo?.name ?? "" })
                    : t("knowledge.groupCreateDescription")
            }
            closeLabel={t("knowledge.groupFormClose")}
            size="md"
            id="poph-formulario-grupo"
            footer={
                <>
                    <SofaButton variant="secondary" disabled={enviando} onClick={aoFechar}>
                        {t("common.cancel")}
                    </SofaButton>
                    <SofaButton
                        variant="primary"
                        type="button"
                        onClick={() => void salvar()}
                        loading={enviando}
                    >
                        {editando
                            ? t("knowledge.saveChanges")
                            : t("knowledge.groupCreateSubmit")}
                    </SofaButton>
                </>
            }
        >
            <form
                id="poph-form-grupo"
                className="poph-pilha"
                style={{ gap: "var(--pop-spacing-lg)" }}
                onSubmit={(evento) => void salvar(evento)}
                noValidate
            >
                {conflito && (
                    <SofaBanner tone="warn" title={t("knowledge.conflictTitle")}>
                        {conflito}
                    </SofaBanner>
                )}
                {erroGeral && (
                    <SofaBanner tone="danger" title={t("knowledge.saveFailedTitle")}>
                        {erroGeral}
                    </SofaBanner>
                )}

                <SofaField
                    label={t("knowledge.nameLabel")}
                    hint={t("knowledge.groupNameHint", { limit: LIMIT_GROUP_NAME })}
                    error={erros.nome}
                    required
                    htmlFor="poph-grupo-nome"
                >
                    {(controle) => (
                        <SofaInput
                            {...semDescribedBy(controle)}
                            value={nome}
                            autoComplete="off"
                            onChange={(evento) => setNome(evento.target.value)}
                        />
                    )}
                </SofaField>

                <SofaField
                    label={t("knowledge.descriptionLabel")}
                    hint={t("knowledge.groupDescriptionHint")}
                    error={erros.descricao}
                    htmlFor="poph-grupo-descricao"
                >
                    {(controle) => (
                        <SofaTextarea
                            {...semDescribedBy(controle)}
                            value={descricao}
                            rows={3}
                            onChange={(evento) => setDescricao(evento.target.value)}
                        />
                    )}
                </SofaField>

                {editando && (
                    <SofaField
                        label={t("knowledge.statusLabel")}
                        hint={t("knowledge.groupStatusHint")}
                        htmlFor="poph-grupo-situacao"
                    >
                        {(controle) => (
                            <SofaSelect
                                {...semDescribedBy(controle)}
                                value={situacao}
                                onChange={(evento) => setSituacao(evento.target.value)}
                            >
                                {SITUACOES.map((opcao) => (
                                    <option key={opcao.valor} value={opcao.valor}>
                                        {t(opcao.labelKey)}
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
