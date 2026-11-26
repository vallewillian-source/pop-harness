
import {
    SofaCheckbox,
    SofaField,
    SofaInput,
    SofaSelect,
    type SofaFieldControlProps,
} from "@sofacoding/sofa-ui";
import { translate, useI18n } from "../i18n";

export type PropertySchema = {
    type?: string;
    title?: string;
    description?: string;
    format?: string;
    enum?: unknown[];
    default?: unknown;
    minimum?: number;
    maximum?: number;
};

export type SchemaObject = {
    type?: string;
    properties?: Record<string, PropertySchema>;
    required?: string[];
};

export type ValuesSchema = Record<string, unknown>;

type Campo = {
    nome: string;
    property: PropertySchema;
    required: boolean;
    secret: boolean;
};

export type StateSecrets = {
    filled?: string[] | null;
    removed?: string[];
};

function withoutDescribedBy(control: SofaFieldControlProps) {
    const { describedBy, ...rest } = control;
    return { ...rest, "aria-describedby": describedBy };
}

export function labelField(nome: string, property: PropertySchema): string {
    if (property.title && property.title.trim() !== "") return property.title;
    const espacado = nome.replace(/[_-]+/g, " ").trim();
    return espacado.charAt(0).toUpperCase() + espacado.slice(1);
}

export function fieldsSchema(
    configuracao: SchemaObject | null | undefined,
    segredos: SchemaObject | null | undefined,
): Campo[] {
    const publicos = Object.entries(configuracao?.properties ?? {}).map(([nome, property]) => ({
        nome,
        property,
        required: (configuracao?.required ?? []).includes(nome),
        secret: property.format === "password",
    }));
    const names = new Set(publicos.map((campo) => campo.nome));
    const ocultos = Object.entries(segredos?.properties ?? {})
        .filter(([nome]) => !names.has(nome))
        .map(([nome, property]) => ({
            nome,
            property,
            required: (segredos?.required ?? []).includes(nome),
            secret: true,
        }));
    return [...publicos, ...ocultos];
}

export function valuesInitials(
    configuracao: SchemaObject,
    segredos: SchemaObject | null,
): ValuesSchema {
    const values: ValuesSchema = {};
    for (const campo of fieldsSchema(configuracao, segredos)) {
        values[campo.nome] = campo.secret ? "" : (campo.property.default ?? "");
    }
    return values;
}

function vazio(value: unknown): boolean {
    return (
        value === undefined || value === null || (typeof value === "string" && value.trim() === "")
    );
}

export function validateValuesSchema(
    configuracao: SchemaObject | null | undefined,
    segredos: SchemaObject | null,
    values: ValuesSchema,
    stateSecrets: StateSecrets = {},
): Record<string, string> {
    const errors: Record<string, string> = {};
    const filled = stateSecrets.filled ?? [];
    const removed = stateSecrets.removed ?? [];
    for (const campo of fieldsSchema(configuracao, segredos)) {
        const value = values[campo.nome];
        const alreadyStored =
            campo.secret && filled.includes(campo.nome) && !removed.includes(campo.nome);
        if (
            campo.required &&
            !alreadyStored &&
            (campo.property.type === "boolean" ? value !== true : vazio(value))
        ) {
            errors[campo.nome] = campo.secret
                ? translate("integrations.schemaValidationSecretRequired")
                : translate("integrations.schemaValidationFieldRequired");
            continue;
        }
        const options = campo.property.enum;
        if (options && !vazio(value) && !options.some((option) => String(option) === String(value))) {
            errors[campo.nome] = translate("integrations.schemaValidationEnum", {
                options: options.map(String).join(", "),
            });
            continue;
        }
        const numerico =
            campo.property.type === "number" || campo.property.type === "integer";
        if (numerico && !vazio(value)) {
            const numero = Number(value);
            if (Number["isNaN"](numero)) {
                errors[campo.nome] = translate("integrations.schemaValidationNumber");
                continue;
            }
            if (campo.property.type === "integer" && !Number.isInteger(numero)) {
                errors[campo.nome] = translate("integrations.schemaValidationInteger");
                continue;
            }
            if (campo.property.minimum !== undefined && numero < campo.property.minimum) {
                errors[campo.nome] = translate("integrations.schemaValidationMin", {
                    min: campo.property.minimum,
                });
                continue;
            }
            if (campo.property.maximum !== undefined && numero > campo.property.maximum) {
                errors[campo.nome] = translate("integrations.schemaValidationMax", {
                    max: campo.property.maximum,
                });
            }
        }
    }
    return errors;
}

export function valuesConfiguration(
    configuracao: SchemaObject,
    values: ValuesSchema,
): Record<string, unknown> {
    const configurationSent: Record<string, unknown> = {};
    for (const campo of fieldsSchema(configuracao, null)) {
        if (campo.secret) continue;
        const value = values[campo.nome];
        if (campo.property.type === "boolean") {
            configurationSent[campo.nome] = value === true;
            continue;
        }
        if (vazio(value)) continue;
        if (campo.property.type === "number" || campo.property.type === "integer") {
            const numero = Number(value);
            if (Number.isFinite(numero)) configurationSent[campo.nome] = numero;
            continue;
        }
        configurationSent[campo.nome] = String(value);
    }
    return configurationSent;
}

export function valuesSecrets(
    segredos: SchemaObject | null,
    values: ValuesSchema,
    removed: string[] = [],
): Record<string, string> {
    const sent: Record<string, string> = {};
    for (const nome of Object.keys(segredos?.properties ?? {})) {
        if (removed.includes(nome)) {
            sent[nome] = "";
            continue;
        }
        const value = values[nome];
        if (vazio(value)) continue;
        sent[nome] = String(value);
    }
    return sent;
}

export function valuesIntegration(
    configuracao: SchemaObject,
    segredos: SchemaObject | null,
    stored: Record<string, unknown> | null | undefined,
): ValuesSchema {
    return { ...valuesInitials(configuracao, segredos), ...(stored ?? {}) };
}

export type FormSchemaProps = {
    configuracao: SchemaObject;
    segredos?: SchemaObject | null;
    values: ValuesSchema;
    onChange: (nome: string, value: unknown) => void;
    errors?: Record<string, string>;
    idPrefix: string;
    onlyRead?: boolean;
    secretsState?: StateSecrets;
    onToggleRemoval?: (nome: string, remove: boolean) => void;
};

export function FormSchema({
    configuracao,
    segredos = null,
    values,
    onChange,
    errors = {},
    idPrefix,
    onlyRead = false,
    secretsState: stateSecrets = {},
    onToggleRemoval,
}: FormSchemaProps) {
    const { translate: t } = useI18n();
    const campos = fieldsSchema(configuracao, segredos);
    const filled = stateSecrets.filled ?? [];
    const removed = stateSecrets.removed ?? [];
    if (campos.length === 0) {
        return (
            <p className="poph-mono-muted" style={{ margin: 0 }}>
                {t("integrations.schemaEmptyManifest")}
            </p>
        );
    }

    return (
        <div className="poph-stack">
            {campos.map((campo) => {
                const id = `${idPrefix}-${campo.nome}`;
                const label = labelField(campo.nome, campo.property);
                const hint = campo.property.description;
                const error = errors[campo.nome];
                const value = values[campo.nome];
                const options = campo.property.enum;
                const alreadyStored = campo.secret && filled.includes(campo.nome);
                const unreadable = campo.secret && stateSecrets.filled === null;
                const removing = campo.secret && removed.includes(campo.nome);
                const hintSecret = !campo.secret
                    ? hint
                    : removing
                      ? t("integrations.schemaSecretRemovingHint")
                      : alreadyStored
                        ? t("integrations.schemaSecretSavedHint")
                        : unreadable
                          ? t("integrations.schemaSecretUnreadableHint")
                          : hint;

                if (campo.property.type === "boolean") {
                    return (
                        <SofaCheckbox
                            key={campo.nome}
                            label={label}
                            description={hint}
                            checked={value === true}
                            disabled={onlyRead}
                            error={!!error}
                            onChange={(evento) => onChange(campo.nome, evento.target.checked)}
                        />
                    );
                }

                return (
                    <div key={campo.nome} className="poph-stack" style={{ gap: 4 }}>
                        <SofaField
                            label={label}
                            hint={hintSecret}
                            error={error}
                            required={campo.required && !alreadyStored}
                            htmlFor={id}
                        >
                            {(control) => {
                                const connections = withoutDescribedBy(control);
                                if (options) {
                                    return (
                                        <SofaSelect
                                            {...connections}
                                            disabled={onlyRead}
                                            value={typeof value === "string" ? value : ""}
                                            onChange={(evento) =>
                                                onChange(campo.nome, evento.target.value)
                                            }
                                        >
                                            <option value="">{t("integrations.schemaSelectChoose")}</option>
                                            {options.map((option) => (
                                                <option key={String(option)} value={String(option)}>
                                                    {String(option)}
                                                </option>
                                            ))}
                                        </SofaSelect>
                                    );
                                }
                                const numerico =
                                    campo.property.type === "number" ||
                                    campo.property.type === "integer";
                                return (
                                    <SofaInput
                                        {...connections}
                                        type={
                                            campo.secret ? "password" : numerico ? "number" : "text"
                                        }
                                        inputMode={numerico ? "decimal" : undefined}
                                        autoComplete={campo.secret ? "new-password" : "off"}
                                        disabled={onlyRead || removing}
                                        placeholder={
                                            campo.secret
                                                ? alreadyStored
                                                    ? t("integrations.schemaSecretSavedPlaceholder")
                                                    : unreadable
                                                      ? t("integrations.schemaSecretUnreadablePlaceholder")
                                                      : t("integrations.schemaSecretDefaultPlaceholder")
                                                : undefined
                                        }
                                        value={
                                            value === undefined || value === null
                                                ? ""
                                                : String(value)
                                        }
                                        onChange={(evento) =>
                                            onChange(campo.nome, evento.target.value)
                                        }
                                    />
                                );
                            }}
                        </SofaField>
                        {campo.secret && alreadyStored && !campo.required && onToggleRemoval && (
                            <SofaCheckbox
                                label={t("integrations.schemaSecretRemoveCheckbox")}
                                checked={removing}
                                onChange={(evento) =>
                                    onToggleRemoval(campo.nome, evento.target.checked)
                                }
                            />
                        )}
                        {campo.secret && alreadyStored && campo.required && (
                            <span className="poph-mono-muted">
                                {t("integrations.schemaSecretRequiredNote")}
                            </span>
                        )}
                        {campo.secret && !alreadyStored && unreadable && !onlyRead && (
                            <span className="poph-mono-muted">
                                {t("integrations.schemaSecretUnreadableNote")}
                            </span>
                        )}
                    </div>
                );
            })}
        </div>
    );
}
