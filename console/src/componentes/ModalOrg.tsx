/**
 * Formulário de organização (criar e editar), em modal.
 *
 * Contrato (docs/API.md):
 * - criar: `POST /admin/v1/orgs` — nome obrigatório, slug opcional (derivado do nome sem ele);
 * - editar: `PATCH /admin/v1/orgs/{id}` com `If-Match: <versão lida>`.
 *
 * Concorrência: um `412` significa que outra pessoa salvou primeiro. A tela **relê** a organização,
 * passa a usar a versão atual e **mantém o que o operador digitou**, para ele revisar e salvar de
 * novo — nada do formulário é descartado.
 */

import { useEffect, useState, type FormEvent } from "react";
import {
    SofaBanner,
    SofaButton,
    SofaField,
    SofaInput,
    SofaModal,
    SofaSelect,
    type SofaFieldControlProps,
} from "@sofacoding/sofa-ui";
import { ErroApi, type Org } from "../api/cliente";
import {
    atualizarOrg,
    comRetencao,
    conflitoDeVersao,
    criarOrg,
    obterOrg,
    retencaoEmDias,
} from "../api/orgs";

const SLUG = /^[a-z0-9-]{3,64}$/;

const SITUACOES = [
    { valor: "active", rotulo: "Ativa" },
    { valor: "inactive", rotulo: "Inativa" },
    { valor: "archived", rotulo: "Arquivada" },
];

export type ModalOrgProps = {
    aberto: boolean;
    aoFechar: () => void;
    /** Organização em edição; `null` cria uma nova. */
    org: Org | null;
    /** Chamado com a organização devolvida pela API (criada ou alterada). */
    aoSalvar: (org: Org) => void;
};

/** As ligações do `SofaField` menos `describedBy`, que vira `aria-describedby` no controle. */
function semDescribedBy(controle: SofaFieldControlProps) {
    const { describedBy, ...resto } = controle;
    return { ...resto, "aria-describedby": describedBy };
}

/** Campo do contrato → campo do formulário. */
const CAMPOS_DA_API: Record<string, keyof Erros> = {
    name: "nome",
    slug: "slug",
    config: "retencao",
    retention_days: "retencao",
};

type Erros = {
    nome?: string;
    slug?: string;
    retencao?: string;
};

export function ModalOrg({ aberto, aoFechar, org, aoSalvar }: ModalOrgProps) {
    const editando = org !== null;
    const [nome, setNome] = useState("");
    const [slug, setSlug] = useState("");
    const [situacao, setSituacao] = useState("active");
    const [retencao, setRetencao] = useState("");
    const [versao, setVersao] = useState(0);
    const [erros, setErros] = useState<Erros>({});
    const [erroGeral, setErroGeral] = useState<string | null>(null);
    const [conflito, setConflito] = useState<string | null>(null);
    const [enviando, setEnviando] = useState(false);

    useEffect(() => {
        if (!aberto) return;
        setNome(org?.name ?? "");
        setSlug(org?.slug ?? "");
        setSituacao(org?.status ?? "active");
        const dias = org ? retencaoEmDias(org) : undefined;
        setRetencao(dias === undefined ? "" : String(dias));
        setVersao(org?.version ?? 0);
        setErros({});
        setErroGeral(null);
        setConflito(null);
        setEnviando(false);
    }, [aberto, org]);

    function validar(): Erros {
        const novos: Erros = {};
        if (!nome.trim()) {
            novos.nome = "informe o nome da organização";
        } else if (nome.trim().length > 120) {
            novos.nome = "o nome aceita até 120 caracteres";
        }
        if (!editando && slug.trim() && !SLUG.test(slug.trim())) {
            novos.slug = "use de 3 a 64 caracteres: letras minúsculas, números e hífen";
        }
        if (retencao.trim()) {
            const dias = Number(retencao);
            if (!Number.isInteger(dias) || dias < 0) {
                novos.retencao = "informe um número inteiro de dias (0 = nunca apagar)";
            }
        }
        return novos;
    }

    /** Aplica o erro da API no campo que ele aponta, quando o contrato informa. */
    function mostrarErroDaApi(falha: ErroApi, criar: boolean) {
        const detalhes = falha.detalhes as Record<string, unknown> | null | undefined;
        const campo = typeof detalhes?.["field"] === "string" ? detalhes["field"] : undefined;
        if (falha.status === 409 || falha.codigo === "conflict") {
            if (criar) {
                setErros({ slug: "já existe uma organização com este slug" });
            } else {
                setErroGeral(falha.message);
            }
            return;
        }
        // O contrato usa o nome do campo em inglês (`name`, `slug`, `config`); o formulário, em
        // português. O mapeamento evita erro que chega e não aparece em campo nenhum.
        const alvo = campo ? CAMPOS_DA_API[campo] : undefined;
        if (alvo) {
            setErros({ [alvo]: falha.message });
            return;
        }
        setErroGeral(falha.message);
    }

    async function salvar(evento?: FormEvent) {
        evento?.preventDefault();
        const novos = validar();
        setErros(novos);
        setErroGeral(null);
        setConflito(null);
        if (Object.keys(novos).length > 0) return;

        const dias = retencao.trim() ? Number(retencao) : null;
        setEnviando(true);
        try {
            if (!org) {
                const config: Record<string, unknown> = {};
                if (dias !== null) config["retention_days"] = dias;
                const criada = await criarOrg({
                    name: nome.trim(),
                    ...(slug.trim() ? { slug: slug.trim() } : {}),
                    config,
                });
                aoSalvar(criada);
                aoFechar();
                return;
            }

            const alterada = await atualizarOrg(org.id, versao, {
                name: nome.trim(),
                status: situacao,
                config: comRetencao(org.config as Record<string, unknown>, dias),
            });
            aoSalvar(alterada);
            aoFechar();
        } catch (falha) {
            const disputa = conflitoDeVersao(falha);
            if (disputa && org) {
                // Relê para continuar de uma versão válida, sem tocar no que foi digitado.
                let versaoAtual = disputa.atual;
                try {
                    const atual = await obterOrg(org.id);
                    versaoAtual = atual.version;
                } catch {
                    // Sem a releitura, a próxima tentativa devolve 412 de novo — mas nada se perde.
                }
                if (typeof versaoAtual === "number") setVersao(versaoAtual);
                setConflito(
                    `Outra pessoa salvou esta organização antes de você${
                        typeof versaoAtual === "number" ? ` (agora na versão ${versaoAtual})` : ""
                    }. O que você digitou foi mantido: revise e salve de novo.`,
                );
                return;
            }
            if (falha instanceof ErroApi) {
                mostrarErroDaApi(falha, !editando);
                return;
            }
            setErroGeral("falha inesperada ao salvar a organização");
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
            title={editando ? "Editar organização" : "Nova organização"}
            description={
                editando
                    ? `Alterando ${org?.name}. O salvamento usa a versão lida da organização.`
                    : "Cada organização é um tenant isolado, com dados e limites próprios."
            }
            closeLabel="Fechar formulário de organização"
            size="md"
            id="poph-formulario-org"
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
                        {editando ? "Salvar alterações" : "Criar organização"}
                    </SofaButton>
                </>
            }
        >
            <form
                id="poph-form-org"
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
                    label="Nome"
                    hint="Como a organização aparece no Console e nos relatórios."
                    error={erros.nome}
                    required
                    htmlFor="poph-org-nome"
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
                    label="Slug"
                    hint={
                        editando
                            ? "O slug é o identificador da URL e não muda depois da criação."
                            : "Identificador na URL (letras minúsculas, números e hífen). Em branco, é derivado do nome."
                    }
                    error={erros.slug}
                    htmlFor="poph-org-slug"
                >
                    {(controle) => (
                        <SofaInput
                            {...semDescribedBy(controle)}
                            value={slug}
                            disabled={editando}
                            autoComplete="off"
                            spellCheck={false}
                            className="poph-mono-mudo"
                            onChange={(evento) => setSlug(evento.target.value)}
                        />
                    )}
                </SofaField>

                {editando && (
                    <SofaField
                        label="Situação"
                        hint="Inativa e arquivada deixam de atender; o histórico continua no banco."
                        htmlFor="poph-org-situacao"
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

                <SofaField
                    label="Retenção do histórico (dias)"
                    hint="Quantos dias as conversas ficam guardadas. 0 = nunca apagar. Em branco, vale o padrão da instância."
                    error={erros.retencao}
                    htmlFor="poph-org-retencao"
                >
                    {(controle) => (
                        <SofaInput
                            {...semDescribedBy(controle)}
                            type="number"
                            min={0}
                            step={1}
                            inputMode="numeric"
                            value={retencao}
                            onChange={(evento) => setRetencao(evento.target.value)}
                        />
                    )}
                </SofaField>
            </form>
        </SofaModal>
    );
}
