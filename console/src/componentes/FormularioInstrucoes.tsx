/**
 * Formulário estruturado das instruções do agente (UX_SPEC §3.5, aba 1 — HR-40).
 *
 * Totalmente controlado: quem guarda o rascunho é a tela, que também calcula as alterações não
 * salvas. Aqui ficam só os campos e a forma do JSON — o mesmo `StructuredInstructions` que o
 * servidor compõe em markdown.
 *
 * Seis blocos, na ordem da especificação: identidade, princípios, escopo, tom, ferramentas e
 * instruções extras. Em todo campo há rótulo (`SofaField` liga `label[for]`), erro no próprio campo
 * e teclado nativo.
 */

import {
    IconPlus,
    IconTrash,
    SofaButton,
    SofaField,
    SofaInput,
    SofaSelect,
    SofaSwitch,
    SofaTag,
    SofaTextarea,
    type SofaFieldControlProps,
} from "@sofacoding/sofa-ui";
import {
    ACOES_DE_CONTINGENCIA,
    EMOJIS,
    ESTILOS,
    FERRAMENTAS,
    FORMATOS,
    IDIOMAS,
    NOMES_DE_FERRAMENTA,
    type Estruturadas,
} from "../api/instrucoes";
import { SecaoDoAgente } from "./AbasDoAgente";

/** Ligações do `SofaField` sem `describedBy`, que vira `aria-describedby` no controle. */
function semDescribedBy(controle: SofaFieldControlProps) {
    const { describedBy, ...resto } = controle;
    return { ...resto, "aria-describedby": describedBy };
}

export type ErrosDoFormulario = {
    nome?: string;
    principios?: string;
    ferramentas?: string;
};

export type FormularioInstrucoesProps = {
    valor: Estruturadas;
    aoMudar: (novo: Estruturadas) => void;
    erros: ErrosDoFormulario;
    desabilitado?: boolean;
};

export function FormularioInstrucoes({
    valor,
    aoMudar,
    erros,
    desabilitado = false,
}: FormularioInstrucoesProps) {
    const identidade = valor.identity;
    const tom = valor.tone;
    const escopo = valor.scope;

    function mudarIdentidade(campo: keyof Estruturadas["identity"], texto: string) {
        aoMudar({ ...valor, identity: { ...identidade, [campo]: texto } });
    }

    function mudarTom(campo: keyof Estruturadas["tone"], texto: string) {
        aoMudar({ ...valor, tone: { ...tom, [campo]: texto } });
    }

    function mudarEscopo(campo: "does" | "does_not", texto: string) {
        aoMudar({ ...valor, scope: { ...escopo, [campo]: texto } });
    }

    function mudarPrincipio(indice: number, mudanca: Partial<Estruturadas["principles"][number]>) {
        const principles = valor.principles.map((principio, posicao) =>
            posicao === indice ? { ...principio, ...mudanca } : principio,
        );
        aoMudar({ ...valor, principles });
    }

    function removerPrincipio(indice: number) {
        aoMudar({
            ...valor,
            principles: valor.principles.filter((_, posicao) => posicao !== indice),
        });
    }

    function adicionarPrincipio() {
        aoMudar({
            ...valor,
            principles: [...valor.principles, { rule: "", fallback_action: "say_dont_know" }],
        });
    }

    function mudarFerramenta(
        indice: number,
        mudanca: Partial<Estruturadas["tools"][number]>,
    ) {
        const tools = valor.tools.map((ferramenta, posicao) =>
            posicao === indice ? { ...ferramenta, ...mudanca } : ferramenta,
        );
        aoMudar({ ...valor, tools });
    }

    /** Liga/desliga uma ferramenta da lista fixa, acrescentando-a se ainda não estiver no JSON. */
    function alternarFerramenta(nome: string, ligada: boolean) {
        const indice = valor.tools.findIndex((ferramenta) => ferramenta.name === nome);
        if (indice >= 0) {
            mudarFerramenta(indice, { enabled: ligada });
            return;
        }
        aoMudar({
            ...valor,
            tools: [...valor.tools, { name: nome, enabled: ligada, when_to_use: "" }],
        });
    }

    const ferramentaPorNome = new Map(valor.tools.map((ferramenta, indice) => [ferramenta.name, indice]));
    const desconhecidas = valor.tools.filter(
        (ferramenta) => !NOMES_DE_FERRAMENTA.includes(ferramenta.name),
    );

    return (
        <div className="poph-instrucoes__formulario">
            <SecaoDoAgente
                titulo="Identidade"
                descricao="Quem é o assistente. O nome e a empresa viram o cabeçalho do texto composto."
            >
                <div className="poph-pilha">
                <div className="poph-grade">
                    <SofaField
                        label="Nome do assistente"
                        hint="Como o agente se apresenta ao cliente."
                        error={erros.nome}
                        required
                        htmlFor="poph-inst-nome"
                    >
                        {(controle) => (
                            <SofaInput
                                {...semDescribedBy(controle)}
                                value={identidade.name}
                                autoComplete="off"
                                disabled={desabilitado}
                                onChange={(evento) => mudarIdentidade("name", evento.target.value)}
                            />
                        )}
                    </SofaField>
                    <SofaField label="Papel ou função" hint="Ex.: atendente, recepcionista.">
                        {(controle) => (
                            <SofaInput
                                {...semDescribedBy(controle)}
                                value={identidade.role}
                                autoComplete="off"
                                disabled={desabilitado}
                                onChange={(evento) => mudarIdentidade("role", evento.target.value)}
                            />
                        )}
                    </SofaField>
                    <SofaField label="Empresa" hint="Nome do negócio do cliente.">
                        {(controle) => (
                            <SofaInput
                                {...semDescribedBy(controle)}
                                value={identidade.company}
                                autoComplete="off"
                                disabled={desabilitado}
                                onChange={(evento) => mudarIdentidade("company", evento.target.value)}
                            />
                        )}
                    </SofaField>
                </div>
                <SofaField
                    label="Descrição da empresa"
                    hint="Um parágrafo sobre o negócio; entra logo abaixo do cabeçalho."
                >
                    {(controle) => (
                        <SofaTextarea
                            {...semDescribedBy(controle)}
                            value={identidade.company_description}
                            rows={3}
                            disabled={desabilitado}
                            onChange={(evento) =>
                                mudarIdentidade("company_description", evento.target.value)
                            }
                        />
                    )}
                </SofaField>
                </div>
            </SecaoDoAgente>

            <SecaoDoAgente
                titulo="Princípios e regras invioláveis"
                descricao="Cada regra entra numerada no texto, com a ação de contingência logo abaixo."
            >
                {erros.principios && (
                    <p role="alert" className="poph-erro-de-secao">
                        {erros.principios}
                    </p>
                )}
                {valor.principles.length === 0 ? (
                    <p className="poph-mono-mudo">
                        Nenhum princípio. Regras como "nunca informar preço fora do catálogo" entram
                        aqui.
                    </p>
                ) : (
                    <ol className="poph-lista-principios">
                        {valor.principles.map((principio, indice) => (
                            <li className="poph-principio" key={`principio-${indice}`}>
                                <SofaField
                                    label={`Regra ${indice + 1}`}
                                    error={
                                        erros.principios && !principio.rule.trim()
                                            ? "escreva a regra ou remova este princípio"
                                            : undefined
                                    }
                                >
                                    {(controle) => (
                                        <SofaInput
                                            {...semDescribedBy(controle)}
                                            value={principio.rule}
                                            autoComplete="off"
                                            disabled={desabilitado}
                                            onChange={(evento) =>
                                                mudarPrincipio(indice, { rule: evento.target.value })
                                            }
                                        />
                                    )}
                                </SofaField>
                                <SofaField label="Quando não puder cumprir">
                                    {(controle) => (
                                        <SofaSelect
                                            {...semDescribedBy(controle)}
                                            value={principio.fallback_action}
                                            disabled={desabilitado}
                                            onChange={(evento) =>
                                                mudarPrincipio(indice, {
                                                    fallback_action: evento.target.value,
                                                })
                                            }
                                        >
                                            {ACOES_DE_CONTINGENCIA.map((acao) => (
                                                <option key={acao.valor} value={acao.valor}>
                                                    {acao.rotulo}
                                                </option>
                                            ))}
                                        </SofaSelect>
                                    )}
                                </SofaField>
                                <SofaButton
                                    variant="subtle"
                                    size="sm"
                                    icon={<IconTrash aria-hidden="true" />}
                                    disabled={desabilitado}
                                    aria-label={`Remover regra ${indice + 1}`}
                                    onClick={() => removerPrincipio(indice)}
                                >
                                    Remover
                                </SofaButton>
                            </li>
                        ))}
                    </ol>
                )}
                <div>
                    <SofaButton
                        variant="secondary"
                        icon={<IconPlus aria-hidden="true" />}
                        disabled={desabilitado}
                        onClick={adicionarPrincipio}
                    >
                        Adicionar princípio
                    </SofaButton>
                </div>
            </SecaoDoAgente>

            <SecaoDoAgente
                titulo="Escopo"
                descricao="O que o agente faz e o que ele não faz. Entra como duas frases no texto."
            >
                <SofaField label="O que o agente faz">
                    {(controle) => (
                        <SofaTextarea
                            {...semDescribedBy(controle)}
                            value={escopo.does}
                            rows={3}
                            disabled={desabilitado}
                            onChange={(evento) => mudarEscopo("does", evento.target.value)}
                        />
                    )}
                </SofaField>
                <SofaField label="O que o agente NÃO faz">
                    {(controle) => (
                        <SofaTextarea
                            {...semDescribedBy(controle)}
                            value={escopo.does_not}
                            rows={3}
                            disabled={desabilitado}
                            onChange={(evento) => mudarEscopo("does_not", evento.target.value)}
                        />
                    )}
                </SofaField>
            </SecaoDoAgente>

            <SecaoDoAgente
                titulo="Tom e comunicação"
                descricao="Sempre entra no texto composto, mesmo com o resto vazio."
            >
                <div className="poph-grade">
                    <SofaField label="Idioma">
                        {(controle) => (
                            <SofaSelect
                                {...semDescribedBy(controle)}
                                value={tom.language}
                                disabled={desabilitado}
                                onChange={(evento) => mudarTom("language", evento.target.value)}
                            >
                                {IDIOMAS.map((idioma) => (
                                    <option key={idioma.valor} value={idioma.valor}>
                                        {idioma.rotulo}
                                    </option>
                                ))}
                            </SofaSelect>
                        )}
                    </SofaField>
                    <SofaField label="Estilo">
                        {(controle) => (
                            <SofaSelect
                                {...semDescribedBy(controle)}
                                value={tom.style}
                                disabled={desabilitado}
                                onChange={(evento) => mudarTom("style", evento.target.value)}
                            >
                                {ESTILOS.map((estilo) => (
                                    <option key={estilo.valor} value={estilo.valor}>
                                        {estilo.rotulo}
                                    </option>
                                ))}
                            </SofaSelect>
                        )}
                    </SofaField>
                    <SofaField label="Emojis">
                        {(controle) => (
                            <SofaSelect
                                {...semDescribedBy(controle)}
                                value={tom.emoji_policy}
                                disabled={desabilitado}
                                onChange={(evento) => mudarTom("emoji_policy", evento.target.value)}
                            >
                                {EMOJIS.map((emoji) => (
                                    <option key={emoji.valor} value={emoji.valor}>
                                        {emoji.rotulo}
                                    </option>
                                ))}
                            </SofaSelect>
                        )}
                    </SofaField>
                    <SofaField label="Formato da resposta">
                        {(controle) => (
                            <SofaSelect
                                {...semDescribedBy(controle)}
                                value={tom.format}
                                disabled={desabilitado}
                                onChange={(evento) => mudarTom("format", evento.target.value)}
                            >
                                {FORMATOS.map((formato) => (
                                    <option key={formato.valor} value={formato.valor}>
                                        {formato.rotulo}
                                    </option>
                                ))}
                            </SofaSelect>
                        )}
                    </SofaField>
                </div>
                <SofaField
                    label="Restrições adicionais"
                    hint="Uma linha de lista no texto composto. Ex.: não prometer prazo fora do catálogo."
                >
                    {(controle) => (
                        <SofaInput
                            {...semDescribedBy(controle)}
                            value={tom.extra_restrictions}
                            autoComplete="off"
                            disabled={desabilitado}
                            onChange={(evento) =>
                                mudarTom("extra_restrictions", evento.target.value)
                            }
                        />
                    )}
                </SofaField>
            </SecaoDoAgente>

            <SecaoDoAgente
                titulo="Ferramentas habilitadas"
                descricao="A lista filtra o que o motor oferece ao modelo. Desligada, a ferramenta não existe naquele turno."
            >
                {erros.ferramentas && (
                    <p role="alert" className="poph-erro-de-secao">
                        {erros.ferramentas}
                    </p>
                )}
                <div className="poph-lista-ferramentas">
                    {FERRAMENTAS.map((ferramenta) => {
                        const indice = ferramentaPorNome.get(ferramenta.nome);
                        const atual = indice === undefined ? undefined : valor.tools[indice];
                        const ligada = atual?.enabled ?? false;
                        return (
                            <div className="poph-ferramenta" key={ferramenta.nome}>
                                <SofaSwitch
                                    label={ferramenta.rotulo}
                                    description={`${ferramenta.descricao} (${ferramenta.nome})`}
                                    checked={ligada}
                                    disabled={desabilitado}
                                    onChange={(evento) =>
                                        alternarFerramenta(ferramenta.nome, evento.target.checked)
                                    }
                                />
                                {ligada && (
                                    <SofaField
                                        label={`Quando usar ${ferramenta.rotulo.toLowerCase()}`}
                                        hint="Opcional: entra depois do nome, no texto composto."
                                    >
                                        {(controle) => (
                                            <SofaInput
                                                {...semDescribedBy(controle)}
                                                value={atual?.when_to_use ?? ""}
                                                autoComplete="off"
                                                disabled={desabilitado}
                                                onChange={(evento) =>
                                                    indice === undefined
                                                        ? undefined
                                                        : mudarFerramenta(indice, {
                                                              when_to_use: evento.target.value,
                                                          })
                                                }
                                            />
                                        )}
                                    </SofaField>
                                )}
                            </div>
                        );
                    })}
                </div>
                {desconhecidas.length > 0 && (
                    <div className="poph-pilha" style={{ gap: 6 }}>
                        <p className="poph-erro-de-secao">
                            Estas ferramentas não existem na V1 e o servidor recusa a gravação
                            enquanto estiverem ligadas:
                        </p>
                        {desconhecidas.map((ferramenta) => (
                            <div className="poph-ferramenta" key={ferramenta.name}>
                                <SofaTag tier="strong">{ferramenta.name} · fora da V1</SofaTag>
                                <SofaSwitch
                                    label={`Manter ${ferramenta.name}`}
                                    description="Desligue para salvar sem ela."
                                    checked={ferramenta.enabled}
                                    disabled={desabilitado}
                                    onChange={(evento) => {
                                        const posicao = valor.tools.findIndex(
                                            (item) => item.name === ferramenta.name,
                                        );
                                        if (posicao >= 0) {
                                            mudarFerramenta(posicao, {
                                                enabled: evento.target.checked,
                                            });
                                        }
                                    }}
                                />
                            </div>
                        ))}
                    </div>
                )}
            </SecaoDoAgente>

            <SecaoDoAgente
                titulo="Instruções extras"
                descricao="Diretrizes de exceção que não cabem nos blocos acima."
            >
                <SofaField label="Texto livre">
                    {(controle) => (
                        <SofaTextarea
                            {...semDescribedBy(controle)}
                            value={valor.extra_instructions}
                            rows={4}
                            disabled={desabilitado}
                            onChange={(evento) =>
                                aoMudar({ ...valor, extra_instructions: evento.target.value })
                            }
                        />
                    )}
                </SofaField>
            </SecaoDoAgente>
        </div>
    );
}
