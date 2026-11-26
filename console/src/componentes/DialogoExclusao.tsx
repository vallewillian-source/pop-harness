/**
 * Diálogo de exclusão destrutiva (UX_SPEC §4.2).
 *
 * Alta fricção: o botão vermelho só habilita quando o operador digita exatamente o identificador do
 * recurso. Para recursos que podem ter conteúdo (organização com agentes, categorias, departamentos
 * e tokens), há uma confirmação extra: sem ela o servidor recusa com `409 conflict`, e o motivo
 * devolvido pela API aparece tal como veio — nunca um erro genérico.
 */

import { useEffect, useState, type ReactNode } from "react";
import {
    SofaBanner,
    SofaButton,
    SofaCheckbox,
    SofaField,
    SofaInput,
    SofaModal,
    type SofaFieldControlProps,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import { translate, useI18n } from "../i18n";

/** Ligações do `SofaField` sem `describedBy`, que vira `aria-describedby` no controle. */
function semDescribedBy(controle: SofaFieldControlProps) {
    const { describedBy, ...resto } = controle;
    return { ...resto, "aria-describedby": describedBy };
}

export type DialogoExclusaoProps = {
    aberto: boolean;
    aoFechar: () => void;
    /** Título curto ("Excluir organização?"). */
    titulo: string;
    /** O que será afetado, em uma frase. */
    descricao: string;
    /** Texto que o operador precisa digitar para habilitar a exclusão. */
    recurso: string;
    /** Nome do que é digitado, para o rótulo ("slug da organização"). */
    rotuloRecurso: string;
    /** Rótulo do botão de confirmação. */
    rotuloConfirmar: string;
    /** Conteúdo extra no corpo (lista do que será apagado). */
    consequencia?: ReactNode;
    /**
     * Oferece a confirmação de "apagar o conteúdo junto" (`?force`). O `DELETE` sem ela é tentado
     * assim mesmo: se o servidor recusar por vínculo, o motivo real aparece na tela.
     */
    permiteForcar?: boolean;
    /**
     * Com `permiteForcar`, a confirmação extra só aparece depois da primeira recusa `409` (HR-42g).
     * Usado quando a tela não sabe antes se há conteúdo vinculado; a contagem vem da própria recusa.
     */
    forcarSoAposRecusa?: boolean;
    rotuloForcar?: string;
    /** Texto de apoio da confirmação extra (padrão: o da organização). */
    descricaoForcar?: string;
    /** Executa a exclusão. Erro sobe para a tela como veio da API. */
    aoConfirmar: (forcar: boolean) => Promise<void>;
    /** Chamado quando a exclusão termina com sucesso. */
    aoExcluir?: () => void;
};

function mensagemDoErro(erro: unknown): { texto: string; codigo?: string } {
    if (erro instanceof ApiError) {
        return { texto: erro.message, codigo: erro.codigo };
    }
    if (erro instanceof Error) {
        return { texto: erro.message };
    }
    return { texto: translate("orgs.deleteUnexpected") };
}

export function DialogoExclusao({
    aberto,
    aoFechar,
    titulo,
    descricao,
    recurso,
    rotuloRecurso,
    rotuloConfirmar,
    consequencia,
    permiteForcar = false,
    forcarSoAposRecusa = false,
    rotuloForcar,
    descricaoForcar,
    aoConfirmar,
    aoExcluir,
}: DialogoExclusaoProps) {
    const { translate: t } = useI18n();
    const [digitado, setDigitado] = useState("");
    const [forcar, setForcar] = useState(false);
    const [enviando, setEnviando] = useState(false);
    const [erro, setErro] = useState<{ texto: string; codigo?: string } | null>(null);
    const [recusouPorVinculo, setRecusouPorVinculo] = useState(false);

    // Cada abertura começa do zero: nada da tentativa anterior fica na tela.
    useEffect(() => {
        if (aberto) {
            setDigitado("");
            setForcar(false);
            setEnviando(false);
            setErro(null);
            setRecusouPorVinculo(false);
        }
    }, [aberto]);

    const confere = digitado === recurso;

    async function confirmar() {
        if (!confere || enviando) return;
        setEnviando(true);
        setErro(null);
        try {
            await aoConfirmar(forcar);
            aoExcluir?.();
            aoFechar();
        } catch (falha) {
            const traduzido = mensagemDoErro(falha);
            setErro(traduzido);
            // 409 é a recusa por conteúdo: a confirmação extra passa a ser a saída.
            if (permiteForcar && falha instanceof ApiError && falha.status === 409) {
                setRecusouPorVinculo(true);
            }
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
            title={titulo}
            description={descricao}
            closeLabel={t("orgs.deleteClose")}
            size="sm"
            id="poph-dialogo-exclusao"
            footer={
                <>
                    <SofaButton variant="secondary" disabled={enviando} onClick={aoFechar}>
                        {t("common.cancel")}
                    </SofaButton>
                    <SofaButton
                        variant="danger"
                        loading={enviando}
                        disabled={!confere}
                        onClick={() => void confirmar()}
                    >
                        {rotuloConfirmar}
                    </SofaButton>
                </>
            }
        >
            <div className="poph-pilha" style={{ gap: "var(--pop-spacing-lg)" }}>
                {consequencia}

                {erro && (
                    <SofaBanner tone="danger" title={t("orgs.deleteRefusedTitle")}>
                        <span>{erro.texto}</span>
                        {erro.codigo && (
                            <div className="poph-mono-mudo" style={{ marginTop: 4 }}>
                                {t("orgs.deleteCode", { code: erro.codigo })}
                            </div>
                        )}
                        {recusouPorVinculo && (
                            <div style={{ marginTop: 6 }}>{t("orgs.deleteForceHint")}</div>
                        )}
                    </SofaBanner>
                )}

                {permiteForcar && (!forcarSoAposRecusa || recusouPorVinculo) && (
                    <SofaCheckbox
                        label={rotuloForcar ?? t("orgs.deleteForceLabel")}
                        description={descricaoForcar ?? t("orgs.deleteForceDescription")}
                        checked={forcar}
                        error={recusouPorVinculo}
                        onChange={(evento) => setForcar(evento.target.checked)}
                    />
                )}

                <div>
                    <SofaField
                        label={
                            <>
                                {t("orgs.deleteTypePrefix")} <code>{recurso}</code>{" "}
                                {t("orgs.deleteTypeSuffix")}
                            </>
                        }
                        hint={t("orgs.deleteTypeHint", { label: rotuloRecurso })}
                        htmlFor="poph-confirma-exclusao"
                    >
                        {(controle) => (
                            <SofaInput
                                {...semDescribedBy(controle)}
                                value={digitado}
                                autoComplete="off"
                                spellCheck={false}
                                onChange={(evento) => setDigitado(evento.target.value)}
                            />
                        )}
                    </SofaField>
                </div>
            </div>
        </SofaModal>
    );
}
