/**
 * Formulário de categoria e de departamento (criar e editar), em modal (HR-39b).
 *
 * Mesmo molde do `ModalOrg`: criar faz `POST`; editar faz `PATCH` com `If-Match` da versão lida.
 * Um `412` significa que outra pessoa salvou primeiro: a tela relê o registro, passa a usar a
 * versão atual e **mantém o que foi digitado** para o operador revisar.
 *
 * Erro de campo: a API aponta o campo em `details.field`, e o nome varia (`name`/`nome`,
 * `description`/`descricao`, `status`/`situacao`), então o mapeamento aceita os dois.
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
    atualizarEstrutura,
    criarEstrutura,
    obterEstrutura,
    type Registro,
    type TipoEstrutura,
} from "../api/estrutura";

const ROTULOS: Record<TipoEstrutura, { singular: string; artigo: string }> = {
    categoria: { singular: "categoria", artigo: "a" },
    departamento: { singular: "departamento", artigo: "o" },
};

const SITUACOES = [
    { valor: "active", rotulo: "Ativo" },
    { valor: "inactive", rotulo: "Inativo" },
    { valor: "archived", rotulo: "Arquivado" },
];

export type ModalEstruturaProps = {
    tipo: TipoEstrutura;
    org: string;
    aberto: boolean;
    aoFechar: () => void;
    /** Registro em edição; `null` cria um novo. */
    registro: Registro | null;
    /** Chamado com o registro devolvido pela API (criado ou alterado). */
    aoSalvar: (registro: Registro) => void;
};

type Erros = {
    nome?: string;
    descricao?: string;
    situacao?: string;
};

/** Nome do campo da API → campo do formulário. */
const CAMPOS_DA_API: Record<string, keyof Erros> = {
    name: "nome",
    nome: "nome",
    description: "descricao",
    descricao: "descricao",
    status: "situacao",
    situacao: "situacao",
};

/** Ligações do `SofaField` sem `describedBy`, que vira `aria-describedby` no controle. */
function semDescribedBy(controle: SofaFieldControlProps) {
    const { describedBy, ...resto } = controle;
    return { ...resto, "aria-describedby": describedBy };
}

export function ModalEstrutura({
    tipo,
    org,
    aberto,
    aoFechar,
    registro,
    aoSalvar,
}: ModalEstruturaProps) {
    const editando = registro !== null;
    const rotulo = ROTULOS[tipo];
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
        setNome(registro?.name ?? "");
        setDescricao(registro?.description ?? "");
        setSituacao(registro?.status ?? "active");
        setVersao(registro?.version ?? 0);
        setErros({});
        setErroGeral(null);
        setConflito(null);
        setEnviando(false);
    }, [aberto, registro]);

    function validar(): Erros {
        const novos: Erros = {};
        if (!nome.trim()) {
            novos.nome = `informe o nome ${rotulo.artigo === "a" ? "da" : "do"} ${rotulo.singular}`;
        } else if (nome.trim().length > 120) {
            novos.nome = "o nome aceita até 120 caracteres";
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
        setErroGeral(falha.message);
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
            if (!registro) {
                const criado = await criarEstrutura(tipo, org, {
                    name: nome.trim(),
                    description: descricao,
                });
                aoSalvar(criado);
                aoFechar();
                return;
            }

            const alterado = await atualizarEstrutura(tipo, org, registro.id, versao, {
                name: nome.trim(),
                description: descricao,
                status: situacao,
            });
            aoSalvar(alterado);
            aoFechar();
        } catch (falha) {
            if (falha instanceof ErroApi && falha.status === 412 && registro) {
                // Relê para continuar de uma versão válida; o que foi digitado não é tocado.
                let versaoAtual: number | undefined;
                try {
                    const atual = await obterEstrutura(tipo, org, registro.id);
                    versaoAtual = atual.version;
                } catch {
                    // Sem a releitura, a próxima tentativa devolve 412 de novo; nada se perde.
                }
                if (typeof versaoAtual === "number") setVersao(versaoAtual);
                setConflito(
                    `Outra pessoa salvou ${rotulo.artigo === "a" ? "esta" : "este"} ${rotulo.singular} antes de você${
                        typeof versaoAtual === "number" ? ` (agora na versão ${versaoAtual})` : ""
                    }. O que você digitou foi mantido: revise e salve de novo.`,
                );
                return;
            }
            if (falha instanceof ErroApi) {
                mostrarErroDaApi(falha);
                return;
            }
            setErroGeral(`falha inesperada ao salvar ${rotulo.artigo === "a" ? "a" : "o"} ${rotulo.singular}`);
        } finally {
            setEnviando(false);
        }
    }

    const titulo = editando
        ? `Editar ${rotulo.singular}`
        : tipo === "categoria"
          ? "Nova categoria"
          : "Novo departamento";

    return (
        <SofaModal
            open={aberto}
            onOpenChange={(novo) => {
                if (!novo && !enviando) aoFechar();
            }}
            title={titulo}
            description={
                editando
                    ? `Alterando ${registro?.name}. O salvamento usa a versão lida.`
                    : tipo === "categoria"
                      ? "Agrupa os produtos do catálogo e filtra o que cada agente pode oferecer."
                      : "Destino de transferência para atendimento humano e padrão das conversas novas."
            }
            closeLabel={`Fechar formulário de ${rotulo.singular}`}
            size="md"
            id={`poph-formulario-${tipo}`}
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
                        {editando ? "Salvar alterações" : `Criar ${rotulo.singular}`}
                    </SofaButton>
                </>
            }
        >
            <form
                id={`poph-form-${tipo}`}
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
                    hint={`Único na organização, sem diferenciar maiúsculas de minúsculas. Até 120 caracteres.`}
                    error={erros.nome}
                    required
                    htmlFor={`poph-${tipo}-nome`}
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
                    label="Descrição"
                    hint="Opcional. Aparece na lista e na conversa com o operador."
                    error={erros.descricao}
                    htmlFor={`poph-${tipo}-descricao`}
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
                        label="Situação"
                        hint="Inativo e arquivado deixam de valer para os agentes; o cadastro continua no banco."
                        error={erros.situacao}
                        htmlFor={`poph-${tipo}-situacao`}
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
