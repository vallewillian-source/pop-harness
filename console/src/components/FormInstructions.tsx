
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
import { SectionAgent } from "../components/TabsAgent";
import { useI18n } from "../i18n";

function withoutDescribedBy(control: SofaFieldControlProps) {
    const { describedBy, ...rest } = control;
    return { ...rest, "aria-describedby": describedBy };
}

export type ErrorsForm = {
    nome?: string;
    principles?: string;
    tools?: string;
};

export type FormInstructionsProps = {
    value: Structured;
    onChange: (fresh: Structured) => void;
    errors: ErrorsForm;
    disabled?: boolean;
};

export function FormInstructions({
    value,
    onChange,
    errors,
    disabled = false,
}: FormInstructionsProps) {
    const { translate: t } = useI18n();
    const identity = value.identity;
    const tom = value.tone;
    const scope = value.scope;

    function changeIdentity(campo: keyof Structured["identity"], texto: string) {
        onChange({ ...value, identity: { ...identity, [campo]: texto } });
    }

    function changeTone(campo: keyof Structured["tone"], texto: string) {
        onChange({ ...value, tone: { ...tom, [campo]: texto } });
    }

    function changeScope(campo: "does" | "does_not", texto: string) {
        onChange({ ...value, scope: { ...scope, [campo]: texto } });
    }

    function changePrinciple(index: number, change: Partial<Structured["principles"][number]>) {
        const principles = value.principles.map((principle, position) =>
            position === index ? { ...principle, ...change } : principle,
        );
        onChange({ ...value, principles });
    }

    function removePrinciple(index: number) {
        onChange({
            ...value,
            principles: value.principles.filter((_, position) => position !== index),
        });
    }

    function addPrinciple() {
        onChange({
            ...value,
            principles: [...value.principles, { rule: "", fallback_action: "say_dont_know" }],
        });
    }

    function changeTool(
        index: number,
        change: Partial<Structured["tools"][number]>,
    ) {
        const tools = value.tools.map((tool, position) =>
            position === index ? { ...tool, ...change } : tool,
        );
        onChange({ ...value, tools });
    }

    function toggleTool(nome: string, linked: boolean) {
        const index = value.tools.findIndex((tool) => tool.name === nome);
        if (index >= 0) {
            changeTool(index, { enabled: linked });
            return;
        }
        onChange({
            ...value,
            tools: [...value.tools, { name: nome, enabled: linked, when_to_use: "" }],
        });
    }

    const toolByName = new Map(value.tools.map((tool, index) => [tool.name, index]));
    const unknown = value.tools.filter(
        (tool) => !TOOL_NAMES.includes(tool.name),
    );

    return (
        <div className="poph-instructions__form">
            <SectionAgent
                title={t("instructions.sectionIdentity")}
                descricao={t("instructions.sectionIdentityDescription")}
            >
                <div className="poph-stack">
                <div className="poph-grid">
                    <SofaField
                        label={t("instructions.nameLabel")}
                        hint={t("instructions.nameHint")}
                        error={errors.nome}
                        required
                        htmlFor="poph-inst-nome"
                    >
                        {(control) => (
                            <SofaInput
                                {...withoutDescribedBy(control)}
                                value={identity.name}
                                autoComplete="off"
                                disabled={disabled}
                                onChange={(evento) => changeIdentity("name", evento.target.value)}
                            />
                        )}
                    </SofaField>
                    <SofaField label={t("instructions.roleLabel")} hint={t("instructions.roleHint")}>
                        {(control) => (
                            <SofaInput
                                {...withoutDescribedBy(control)}
                                value={identity.role}
                                autoComplete="off"
                                disabled={disabled}
                                onChange={(evento) => changeIdentity("role", evento.target.value)}
                            />
                        )}
                    </SofaField>
                    <SofaField label={t("instructions.companyLabel")} hint={t("instructions.companyHint")}>
                        {(control) => (
                            <SofaInput
                                {...withoutDescribedBy(control)}
                                value={identity.company}
                                autoComplete="off"
                                disabled={disabled}
                                onChange={(evento) => changeIdentity("company", evento.target.value)}
                            />
                        )}
                    </SofaField>
                </div>
                <SofaField
                    label={t("instructions.companyDescriptionLabel")}
                    hint={t("instructions.companyDescriptionHint")}
                >
                    {(control) => (
                        <SofaTextarea
                            {...withoutDescribedBy(control)}
                            value={identity.company_description}
                            rows={3}
                            disabled={disabled}
                            onChange={(evento) =>
                                changeIdentity("company_description", evento.target.value)
                            }
                        />
                    )}
                </SofaField>
                </div>
            </SectionAgent>

            <SectionAgent
                title={t("instructions.sectionPrinciples")}
                descricao={t("instructions.sectionPrinciplesDescription")}
            >
                {errors.principles && (
                    <p role="alert" className="poph-section-error">
                        {errors.principles}
                    </p>
                )}
                {value.principles.length === 0 ? (
                    <p className="poph-mono-muted">{t("instructions.noPrinciples")}</p>
                ) : (
                    <ol className="poph-list-principles">
                        {value.principles.map((principle, index) => (
                            <li className="poph-principle" key={`principio-${index}`}>
                                <SofaField
                                    label={t("instructions.ruleLabel", { number: index + 1 })}
                                    error={
                                        errors.principles && !principle.rule.trim()
                                            ? t("instructions.emptyRuleError")
                                            : undefined
                                    }
                                >
                                    {(control) => (
                                        <SofaInput
                                            {...withoutDescribedBy(control)}
                                            value={principle.rule}
                                            autoComplete="off"
                                            disabled={disabled}
                                            onChange={(evento) =>
                                                changePrinciple(index, { rule: evento.target.value })
                                            }
                                        />
                                    )}
                                </SofaField>
                                <SofaField label={t("instructions.fallbackLabel")}>
                                    {(control) => (
                                        <SofaSelect
                                            {...withoutDescribedBy(control)}
                                            value={principle.fallback_action}
                                            disabled={disabled}
                                            onChange={(evento) =>
                                                changePrinciple(index, {
                                                    fallback_action: evento.target.value,
                                                })
                                            }
                                        >
                                            {CONTINGENCY_ACTIONS.map((acao) => (
                                                <option key={acao.value} value={acao.value}>
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
                                    disabled={disabled}
                                    aria-label={t("instructions.removeRuleLabel", { number: index + 1 })}
                                    onClick={() => removePrinciple(index)}
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
                        disabled={disabled}
                        onClick={addPrinciple}
                    >
                        {t("instructions.addPrinciple")}
                    </SofaButton>
                </div>
            </SectionAgent>

            <SectionAgent
                title={t("instructions.sectionScope")}
                descricao={t("instructions.sectionScopeDescription")}
            >
                <SofaField label={t("instructions.doesLabel")}>
                    {(control) => (
                        <SofaTextarea
                            {...withoutDescribedBy(control)}
                            value={scope.does}
                            rows={3}
                            disabled={disabled}
                            onChange={(evento) => changeScope("does", evento.target.value)}
                        />
                    )}
                </SofaField>
                <SofaField label={t("instructions.doesNotLabel")}>
                    {(control) => (
                        <SofaTextarea
                            {...withoutDescribedBy(control)}
                            value={scope.does_not}
                            rows={3}
                            disabled={disabled}
                            onChange={(evento) => changeScope("does_not", evento.target.value)}
                        />
                    )}
                </SofaField>
            </SectionAgent>

            <SectionAgent
                title={t("instructions.sectionTone")}
                descricao={t("instructions.sectionToneDescription")}
            >
                <div className="poph-grid">
                    <SofaField label={t("instructions.languageLabel")}>
                        {(control) => (
                            <SofaSelect
                                {...withoutDescribedBy(control)}
                                value={tom.language}
                                disabled={disabled}
                                onChange={(evento) => changeTone("language", evento.target.value)}
                            >
                                {IDIOMAS.map((idioma) => (
                                    <option key={idioma.value} value={idioma.value}>
                                        {t(idioma.labelKey)}
                                    </option>
                                ))}
                            </SofaSelect>
                        )}
                    </SofaField>
                    <SofaField label={t("instructions.styleLabel")}>
                        {(control) => (
                            <SofaSelect
                                {...withoutDescribedBy(control)}
                                value={tom.style}
                                disabled={disabled}
                                onChange={(evento) => changeTone("style", evento.target.value)}
                            >
                                {ESTILOS.map((estilo) => (
                                    <option key={estilo.value} value={estilo.value}>
                                        {t(estilo.labelKey)}
                                    </option>
                                ))}
                            </SofaSelect>
                        )}
                    </SofaField>
                    <SofaField label={t("instructions.emojiLabel")}>
                        {(control) => (
                            <SofaSelect
                                {...withoutDescribedBy(control)}
                                value={tom.emoji_policy}
                                disabled={disabled}
                                onChange={(evento) => changeTone("emoji_policy", evento.target.value)}
                            >
                                {EMOJIS.map((emoji) => (
                                    <option key={emoji.value} value={emoji.value}>
                                        {t(emoji.labelKey)}
                                    </option>
                                ))}
                            </SofaSelect>
                        )}
                    </SofaField>
                    <SofaField label={t("instructions.formatLabel")}>
                        {(control) => (
                            <SofaSelect
                                {...withoutDescribedBy(control)}
                                value={tom.format}
                                disabled={disabled}
                                onChange={(evento) => changeTone("format", evento.target.value)}
                            >
                                {FORMATS.map((formato) => (
                                    <option key={formato.value} value={formato.value}>
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
                    {(control) => (
                        <SofaInput
                            {...withoutDescribedBy(control)}
                            value={tom.extra_restrictions}
                            autoComplete="off"
                            disabled={disabled}
                            onChange={(evento) =>
                                changeTone("extra_restrictions", evento.target.value)
                            }
                        />
                    )}
                </SofaField>
            </SectionAgent>

            <SectionAgent
                title={t("instructions.sectionTools")}
                descricao={t("instructions.sectionToolsDescription")}
            >
                {errors.tools && (
                    <p role="alert" className="poph-section-error">
                        {errors.tools}
                    </p>
                )}
                <div className="poph-list-tools">
                    {TOOLS.map((tool) => {
                        const index = toolByName.get(tool.nome);
                        const current = index === undefined ? undefined : value.tools[index];
                        const linked = current?.enabled ?? false;
                        return (
                            <div className="poph-tool" key={tool.nome}>
                                <SofaSwitch
                                    label={t(tool.labelKey)}
                                    description={`${t(tool.descriptionKey)} (${tool.nome})`}
                                    checked={linked}
                                    disabled={disabled}
                                    onChange={(evento) =>
                                        toggleTool(tool.nome, evento.target.checked)
                                    }
                                />
                                {linked && (
                                    <SofaField
                                        label={t("instructions.whenToUseLabel", { tool: t(tool.labelKey).toLowerCase() })}
                                        hint={t("instructions.whenToUseHint")}
                                    >
                                        {(control) => (
                                            <SofaInput
                                                {...withoutDescribedBy(control)}
                                                value={current?.when_to_use ?? ""}
                                                autoComplete="off"
                                                disabled={disabled}
                                                onChange={(evento) =>
                                                    index === undefined
                                                        ? undefined
                                                        : changeTool(index, {
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
                {unknown.length > 0 && (
                    <div className="poph-stack" style={{ gap: 6 }}>
                        <p className="poph-section-error">{t("instructions.unknownToolsWarning")}</p>
                        {unknown.map((tool) => (
                            <div className="poph-tool" key={tool.name}>
                                <SofaTag tier="strong">{t("instructions.toolOutsideV1", { name: tool.name })}</SofaTag>
                                <SofaSwitch
                                    label={t("instructions.keepToolLabel", { name: tool.name })}
                                    description={t("instructions.keepToolDescription")}
                                    checked={tool.enabled}
                                    disabled={disabled}
                                    onChange={(evento) => {
                                        const position = value.tools.findIndex(
                                            (item) => item.name === tool.name,
                                        );
                                        if (position >= 0) {
                                            changeTool(position, {
                                                enabled: evento.target.checked,
                                            });
                                        }
                                    }}
                                />
                            </div>
                        ))}
                    </div>
                )}
            </SectionAgent>

            <SectionAgent
                title={t("instructions.sectionExtra")}
                descricao={t("instructions.sectionExtraDescription")}
            >
                <SofaField label={t("instructions.extraLabel")}>
                    {(control) => (
                        <SofaTextarea
                            {...withoutDescribedBy(control)}
                            value={value.extra_instructions}
                            rows={4}
                            disabled={disabled}
                            onChange={(evento) =>
                                onChange({ ...value, extra_instructions: evento.target.value })
                            }
                        />
                    )}
                </SofaField>
            </SectionAgent>
        </div>
    );
}
