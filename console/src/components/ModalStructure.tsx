
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
import { useI18n } from "../i18n";
import {
    updateStructure,
    createStructure,
    getStructure,
    type Registro,
    type StructureType,
} from "../api/structure";

const SITUATIONS = [
    { value: "active", chave: "structure.statusActive" as const },
    { value: "inactive", chave: "structure.statusInactive" as const },
    { value: "archived", chave: "structure.statusArchived" as const },
];

export type ModalStructureProps = {
    tipo: StructureType;
    org: string;
    opened: boolean;
    onClose: () => void;
    registro: Registro | null;
    onSave: (registro: Registro) => void;
};

type Errors = {
    nome?: string;
    descricao?: string;
    situacao?: string;
};

const FIELDS_API: Record<string, keyof Errors> = {
    name: "nome",
    nome: "nome",
    description: "descricao",
    descricao: "descricao",
    status: "situacao",
    situacao: "situacao",
};

function withoutDescribedBy(control: SofaFieldControlProps) {
    const { describedBy, ...rest } = control;
    return { ...rest, "aria-describedby": describedBy };
}

export function ModalStructure({
    tipo,
    org,
    opened: isOpen,
    onClose,
    registro,
    onSave,
}: ModalStructureProps) {
    const { translate } = useI18n();
    const editing = registro !== null;
    const [nome, setName] = useState("");
    const [descricao, setDescription] = useState("");
    const [situacao, setSituation] = useState("active");
    const [versao, setVersion] = useState(0);
    const [errors, setErrors] = useState<Errors>({});
    const [errorGeneral, setErrorGeneral] = useState<string | null>(null);
    const [conflict, setConflict] = useState<string | null>(null);
    const [sending, setSending] = useState(false);

    useEffect(() => {
        if (!isOpen) return;
        setName(registro?.name ?? "");
        setDescription(registro?.description ?? "");
        setSituation(registro?.status ?? "active");
        setVersion(registro?.version ?? 0);
        setErrors({});
        setErrorGeneral(null);
        setConflict(null);
        setSending(false);
    }, [isOpen, registro]);

    function validar(): Errors {
        const freshItems: Errors = {};
        if (!nome.trim()) {
            freshItems.nome =
                tipo === "categoria"
                    ? translate("structure.validationCategoryNameRequired")
                    : translate("structure.validationDepartmentNameRequired");
        } else if (nome.trim().length > 120) {
            freshItems.nome = translate("structure.validationNameMaxLength");
        }
        return freshItems;
    }

    function showErrorApi(failure: ApiError) {
        const details = failure.details as Record<string, unknown> | null | undefined;
        const campo = typeof details?.["field"] === "string" ? details["field"] : undefined;
        const motivo =
            typeof details?.["reason"] === "string" ? details["reason"] : failure.message;
        const target = campo ? FIELDS_API[campo] : undefined;
        if (target) {
            setErrors({ [target]: motivo });
            return;
        }
        setErrorGeneral(failure.message);
    }

    async function save(evento?: FormEvent) {
        evento?.preventDefault();
        const freshItems = validar();
        setErrors(freshItems);
        setErrorGeneral(null);
        setConflict(null);
        if (Object.keys(freshItems).length > 0) return;

        setSending(true);
        try {
            if (!registro) {
                const created = await createStructure(tipo, org, {
                    name: nome.trim(),
                    description: descricao,
                });
                onSave(created);
                onClose();
                return;
            }

            const changed = await updateStructure(tipo, org, registro.id, versao, {
                name: nome.trim(),
                description: descricao,
                status: situacao,
            });
            onSave(changed);
            onClose();
        } catch (failure) {
            if (failure instanceof ApiError && failure.status === 412 && registro) {
                let versionCurrent: number | undefined;
                try {
                    const current = await getStructure(tipo, org, registro.id);
                    versionCurrent = current.version;
                } catch {
                }
                if (typeof versionCurrent === "number") setVersion(versionCurrent);
                const versionText =
                    typeof versionCurrent === "number"
                        ? translate("structure.conflictVersionSuffix", { version: versionCurrent })
                        : "";
                setConflict(
                    tipo === "categoria"
                        ? translate("structure.conflictCategory", { version: versionText })
                        : translate("structure.conflictDepartment", { version: versionText }),
                );
                return;
            }
            if (failure instanceof ApiError) {
                showErrorApi(failure);
                return;
            }
            setErrorGeneral(
                tipo === "categoria"
                    ? translate("structure.unexpectedSaveCategoryError")
                    : translate("structure.unexpectedSaveDepartmentError"),
            );
        } finally {
            setSending(false);
        }
    }

    const title = editing
        ? tipo === "categoria"
            ? translate("structure.modalTitleEditCategory")
            : translate("structure.modalTitleEditDepartment")
        : tipo === "categoria"
          ? translate("structure.modalTitleNewCategory")
          : translate("structure.modalTitleNewDepartment");

    return (
        <SofaModal
            open={isOpen}
            onOpenChange={(fresh) => {
                if (!fresh && !sending) onClose();
            }}
            title={title}
            description={
                editing
                    ? translate("structure.modalDescriptionEdit", { name: registro?.name ?? "" })
                    : tipo === "categoria"
                      ? translate("structure.modalDescriptionNewCategory")
                      : translate("structure.modalDescriptionNewDepartment")
            }
            closeLabel={
                tipo === "categoria"
                    ? translate("structure.modalCloseCategory")
                    : translate("structure.modalCloseDepartment")
            }
            size="md"
            id={`poph-formulario-${tipo}`}
            footer={
                <>
                    <SofaButton variant="secondary" disabled={sending} onClick={onClose}>
                        {translate("structure.cancel")}
                    </SofaButton>
                    <SofaButton
                        variant="primary"
                        type="button"
                        onClick={() => void save()}
                        loading={sending}
                    >
                        {editing
                            ? translate("structure.saveChanges")
                            : tipo === "categoria"
                              ? translate("structure.createCategory")
                              : translate("structure.createDepartment")}
                    </SofaButton>
                </>
            }
        >
            <form
                id={`poph-form-${tipo}`}
                className="poph-stack"
                style={{ gap: "var(--pop-spacing-lg)" }}
                onSubmit={(evento) => void save(evento)}
                noValidate
            >
                {conflict && (
                    <SofaBanner tone="warn" title={translate("structure.bannerOutdatedTitle")}>
                        {conflict}
                    </SofaBanner>
                )}
                {errorGeneral && (
                    <SofaBanner tone="danger" title={translate("structure.bannerSaveErrorTitle")}>
                        {errorGeneral}
                    </SofaBanner>
                )}

                <SofaField
                    label={translate("structure.fieldNameLabel")}
                    hint={translate("structure.fieldNameHint")}
                    error={errors.nome}
                    required
                    htmlFor={`poph-${tipo}-nome`}
                >
                    {(control) => (
                        <SofaInput
                            {...withoutDescribedBy(control)}
                            value={nome}
                            autoComplete="off"
                            onChange={(evento) => setName(evento.target.value)}
                        />
                    )}
                </SofaField>

                <SofaField
                    label={translate("structure.fieldDescriptionLabel")}
                    hint={translate("structure.fieldDescriptionHint")}
                    error={errors.descricao}
                    htmlFor={`poph-${tipo}-descricao`}
                >
                    {(control) => (
                        <SofaTextarea
                            {...withoutDescribedBy(control)}
                            value={descricao}
                            rows={3}
                            onChange={(evento) => setDescription(evento.target.value)}
                        />
                    )}
                </SofaField>

                {editing && (
                    <SofaField
                        label={translate("structure.fieldStatusLabel")}
                        hint={translate("structure.fieldStatusHint")}
                        error={errors.situacao}
                        htmlFor={`poph-${tipo}-situacao`}
                    >
                        {(control) => (
                            <SofaSelect
                                {...withoutDescribedBy(control)}
                                value={situacao}
                                onChange={(evento) => setSituation(evento.target.value)}
                            >
                                {SITUATIONS.map((option) => (
                                    <option key={option.value} value={option.value}>
                                        {translate(option.chave)}
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
