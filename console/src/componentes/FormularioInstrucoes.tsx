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
    CONTINGENCY_ACTIONS,
    EMOJIS,
    ESTILOS,
    TOOLS,
    FORMATS,
    IDIOMAS,
    TOOL_NAMES,
    type Structured,
} from "../api/instructions";
import { SecaoDoAgente } from "./AbasDoAgente";
import { useI18n } from "../i18n";

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
    valor: Structured;
    aoMudar: (novo: Structured) => void;
    erros: ErrosDoFormulario;
    desabilitado?: boolean;
};

export function FormularioInstrucoes({
    valor,
    aoMudar,
    erros,
    desabilitado = false,
}: FormularioInstrucoesProps) {
    const { translate: t } = useI18n();
    const identidade = valor.identity;
    const tom = valor.tone;
    const escopo = valor.scope;

    function mudarIdentidade(campo: keyof Structured["identity"], texto: string) {
        aoMudar({ ...valor, identity: { ...identidade, [campo]: texto } });
    }

    function mudarTom(campo: keyof Structured["tone"], texto: string) {
        aoMudar({ ...valor, tone: { ...tom, [campo]: texto } });
    }

    function mudarEscopo(campo: "does" | "does_not", texto: string) {
        aoMudar({ ...valor, scope: { ...escopo, [campo]: texto } });
    }

    function mudarPrincipio(indice: number, mudanca: Partial<Structured["principles"][number]>) {
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
        mudanca: Partial<Structured["tools"][number]>,
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
        (ferramenta) => !TOOL_NAMES.includes(ferramenta.name),
    );

    return (
        <div className="poph-instrucoes__formulario">
            <SecaoDoAgente
                titulo={t("instructions.sectionIdentity")}
                descricao={t("instructions.sectionIdentityDescription")}
            >
                <div className="poph-pilha">
                <div className="poph-grade">
                    <SofaField
                        label={t("instructions.nameLabel")}
                        hint={t("instructions.nameHint")}
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
                    <SofaField label={t("instructions.roleLabel")} hint={t("instructions.roleHint")}>
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
                    <SofaField label={t("instructions.companyLabel")} hint={t("instructions.companyHint")}>
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
                    label={t("instructions.companyDescriptionLabel")}
                    hint={t("instructions.companyDescriptionHint")}
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
                titulo={t("instructions.sectionPrinciples")}
                descricao={t("instructions.sectionPrinciplesDescription")}
            >
                {erros.principios && (
                    <p role="alert" className="poph-erro-de-secao">
                        {erros.principios}
                    </p>
                )}
                {valor.principles.length === 0 ? (
                    <p className="poph-mono-mudo">{t("instructions.noPrinciples")}</p>
                ) : (
                    <ol className="poph-lista-principios">
                        {valor.principles.map((principio, indice) => (
                            <li className="poph-principio" key={`principio-${indice}`}>
                                <SofaField
                                    label={t("instructions.ruleLabel", { number: indice + 1 })}
                                    error={
                                        erros.principios && !principio.rule.trim()
                                            ? t("instructions.emptyRuleError")
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
                                <SofaField label={t("instructions.fallbackLabel")}>
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
                                            {CONTINGENCY_ACTIONS.map((acao) => (
                                                <option key={acao.valor} value={acao.valor}>
                                                    {t(acao.labelKey)}
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
                                    aria-label={t("instructions.removeRuleLabel", { number: indice + 1 })}
                                    onClick={() => removerPrincipio(indice)}
                                >
                                    {t("instructions.remove")}
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
                        {t("instructions.addPrinciple")}
                    </SofaButton>
                </div>
            </SecaoDoAgente>

            <SecaoDoAgente
                titulo={t("instructions.sectionScope")}
                descricao={t("instructions.sectionScopeDescription")}
            >
                <SofaField label={t("instructions.doesLabel")}>
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
                <SofaField label={t("instructions.doesNotLabel")}>
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
                titulo={t("instructions.sectionTone")}
                descricao={t("instructions.sectionToneDescription")}
            >
                <div className="poph-grade">
                    <SofaField label={t("instructions.languageLabel")}>
                        {(controle) => (
                            <SofaSelect
                                {...semDescribedBy(controle)}
                                value={tom.language}
                                disabled={desabilitado}
                                onChange={(evento) => mudarTom("language", evento.target.value)}
                            >
                                {IDIOMAS.map((idioma) => (
                                    <option key={idioma.valor} value={idioma.valor}>
                                        {t(idioma.labelKey)}
                                    </option>
                                ))}
                            </SofaSelect>
                        )}
                    </SofaField>
                    <SofaField label={t("instructions.styleLabel")}>
                        {(controle) => (
                            <SofaSelect
                                {...semDescribedBy(controle)}
                                value={tom.style}
                                disabled={desabilitado}
                                onChange={(evento) => mudarTom("style", evento.target.value)}
                            >
                                {ESTILOS.map((estilo) => (
                                    <option key={estilo.valor} value={estilo.valor}>
                                        {t(estilo.labelKey)}
                                    </option>
                                ))}
                            </SofaSelect>
                        )}
                    </SofaField>
                    <SofaField label={t("instructions.emojiLabel")}>
                        {(controle) => (
                            <SofaSelect
                                {...semDescribedBy(controle)}
                                value={tom.emoji_policy}
                                disabled={desabilitado}
                                onChange={(evento) => mudarTom("emoji_policy", evento.target.value)}
                            >
                                {EMOJIS.map((emoji) => (
                                    <option key={emoji.valor} value={emoji.valor}>
                                        {t(emoji.labelKey)}
                                    </option>
                                ))}
                            </SofaSelect>
                        )}
                    </SofaField>
                    <SofaField label={t("instructions.formatLabel")}>
                        {(controle) => (
                            <SofaSelect
                                {...semDescribedBy(controle)}
                                value={tom.format}
                                disabled={desabilitado}
                                onChange={(evento) => mudarTom("format", evento.target.value)}
                            >
                                {FORMATS.map((formato) => (
                                    <option key={formato.valor} value={formato.valor}>
                                        {t(formato.labelKey)}
                                    </option>
                                ))}
                            </SofaSelect>
                        )}
                    </SofaField>
                </div>
                <SofaField
                    label={t("instructions.extraRestrictionsLabel")}
                    hint={t("instructions.extraRestrictionsHint")}
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
                titulo={t("instructions.sectionTools")}
                descricao={t("instructions.sectionToolsDescription")}
            >
                {erros.ferramentas && (
                    <p role="alert" className="poph-erro-de-secao">
                        {erros.ferramentas}
                    </p>
                )}
                <div className="poph-lista-ferramentas">
                    {TOOLS.map((ferramenta) => {
                        const indice = ferramentaPorNome.get(ferramenta.nome);
                        const atual = indice === undefined ? undefined : valor.tools[indice];
                        const ligada = atual?.enabled ?? false;
                        return (
                            <div className="poph-ferramenta" key={ferramenta.nome}>
                                <SofaSwitch
                                    label={t(ferramenta.labelKey)}
                                    description={`${t(ferramenta.descriptionKey)} (${ferramenta.nome})`}
                                    checked={ligada}
                                    disabled={desabilitado}
                                    onChange={(evento) =>
                                        alternarFerramenta(ferramenta.nome, evento.target.checked)
                                    }
                                />
                                {ligada && (
                                    <SofaField
                                        label={t("instructions.whenToUseLabel", { tool: t(ferramenta.labelKey).toLowerCase() })}
                                        hint={t("instructions.whenToUseHint")}
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
                        <p className="poph-erro-de-secao">{t("instructions.unknownToolsWarning")}</p>
                        {desconhecidas.map((ferramenta) => (
                            <div className="poph-ferramenta" key={ferramenta.name}>
                                <SofaTag tier="strong">{t("instructions.toolOutsideV1", { name: ferramenta.name })}</SofaTag>
                                <SofaSwitch
                                    label={t("instructions.keepToolLabel", { name: ferramenta.name })}
                                    description={t("instructions.keepToolDescription")}
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
                titulo={t("instructions.sectionExtra")}
                descricao={t("instructions.sectionExtraDescription")}
            >
                <SofaField label={t("instructions.extraLabel")}>
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
