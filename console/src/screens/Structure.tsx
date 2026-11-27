
import { useCallback, useEffect, useId, useState, type ReactNode } from "react";
import { useParams } from "react-router-dom";
import {
    IconBuilding,
    IconGrid,
    IconPlus,
    IconRefresh,
    SofaButton,
    SofaEmptyState,
    SofaErrorState,
    SofaSkeleton,
} from "@sofacoding/sofa-ui";
import {
    agentsUsing,
    listAgents,
    listStructure,
    type AgentSummary,
    type Category,
    type Departamento,
    type Registro,
    type StructureType,
} from "../api/structure";
import { getOrg } from "../api/orgs";
import { DialogDeleteCategory } from "../components/DialogDeleteCategory";
import { DialogDeleteDepartment } from "../components/DialogDeleteDepartment";
import { ModalStructure } from "../components/ModalStructure";
import { useI18n } from "../i18n";
import "../styles/structure.css";

type Estado = {
    loading: boolean;
    error: string | null;
    items: Registro[];
};

const STATE_INITIAL: Estado = { loading: true, error: null, items: [] };

function classStatus(situacao: string): string {
    return situacao === "active" ? "poph-status--active" : "poph-status--inactive";
}

function labelStatus(
    situacao: string,
    translate: ReturnType<typeof useI18n>["translate"],
): string {
    switch (situacao) {
        case "active":
            return translate("structure.statusActive");
        case "inactive":
            return translate("structure.statusInactive");
        case "archived":
            return translate("structure.statusArchived");
        default:
            return situacao;
    }
}

function textAgents(
    total: number | null,
    translate: ReturnType<typeof useI18n>["translate"],
): string {
    if (total === null) return translate("structure.agentsCountUnknown");
    return total === 1
        ? translate("structure.agentsCountOne")
        : translate("structure.agentsCountMany", { count: total });
}

type ColumnProps = {
    tipo: StructureType;
    org: string;
    agentCount?: (id: string) => number | null;
    onChange?: () => void;
};

function SectionStructure({ tipo, org, agentCount, onChange }: ColumnProps) {
    const { translate } = useI18n();
    const [estado, setState] = useState<Estado>(STATE_INITIAL);
    const [edit, setEdit] = useState<{ opened: boolean; registro: Registro | null }>({
        opened: false,
        registro: null,
    });
    const [toDelete, setToDelete] = useState<Departamento | null>(null);
    const [categoryDelete, setCategoryToDelete] = useState<Category | null>(null);
    const titleId = useId();

    const isDepartment = tipo === "departamento";
    const title = isDepartment
        ? translate("structure.departmentsTitle")
        : translate("structure.categoriesTitle");

    const load = useCallback(async () => {
        setState((current) => ({ ...current, loading: true, error: null }));
        try {
            const items = await listStructure(tipo, org);
            setState({ loading: false, error: null, items: items });
        } catch (failure) {
            const mensagem =
                failure instanceof Error
                    ? failure.message
                    : isDepartment
                      ? translate("structure.loadDepartmentsError")
                      : translate("structure.loadCategoriesError");
            setState({ loading: false, error: mensagem, items: [] });
        }
    }, [tipo, org, isDepartment, translate]);

    useEffect(() => {
        void load();
    }, [load]);

    function onSaveRecord(registro: Registro) {
        setState((current) => {
            const exists = current.items.some((item) => item.id === registro.id);
            const items = exists
                ? current.items.map((item) => (item.id === registro.id ? registro : item))
                : [...current.items, registro];
            return { ...current, items: items };
        });
        onChange?.();
        void load();
    }

    const buttonNew = (
        <SofaButton
            variant="primary"
            icon={<IconPlus aria-hidden="true" />}
            onClick={() => setEdit({ opened: true, registro: null })}
        >
            {isDepartment
                ? translate("structure.modalTitleNewDepartment")
                : translate("structure.modalTitleNewCategory")}
        </SofaButton>
    );

    let content: ReactNode;
    if (estado.loading && estado.items.length === 0) {
        content = (
            <div
                aria-busy="true"
                aria-label={
                    isDepartment
                        ? translate("structure.loadingDepartments")
                        : translate("structure.loadingCategories")
                }
            >
                <SofaSkeleton lines={3} />
            </div>
        );
    } else if (estado.error) {
        content = (
            <SofaErrorState
                title={
                    isDepartment
                        ? translate("structure.errorDepartmentsTitle")
                        : translate("structure.errorCategoriesTitle")
                }
                description={translate("structure.errorDescription")}
                details={estado.error}
                retry={{ label: translate("structure.retry"), onClick: () => void load() }}
            />
        );
    } else if (estado.items.length === 0) {
        content = (
            <SofaEmptyState
                icon={isDepartment ? <IconBuilding aria-hidden="true" /> : <IconGrid aria-hidden="true" />}
                title={
                    isDepartment
                        ? translate("structure.emptyDepartmentsTitle")
                        : translate("structure.emptyCategoriesTitle")
                }
                description={
                    isDepartment
                        ? translate("structure.emptyDepartmentsDescription")
                        : translate("structure.emptyCategoriesDescription")
                }
                action={
                    <SofaButton
                        variant="primary"
                        icon={<IconPlus aria-hidden="true" />}
                        onClick={() => setEdit({ opened: true, registro: null })}
                    >
                        {isDepartment
                            ? translate("structure.createFirstDepartment")
                            : translate("structure.createFirstCategory")}
                    </SofaButton>
                }
            />
        );
    } else {
        content = (
            <ul
                className="poph-structure__list"
                aria-label={
                    isDepartment
                        ? translate("structure.listDepartmentsAria")
                        : translate("structure.listCategoriesAria")
                }
            >
                {estado.items.map((item) => (
                    <li className="poph-structure__line" key={item.id}>
                        <div className="poph-structure__text">
                            <span className="poph-structure__name">{item.name}</span>
                            <span className="poph-structure__description">
                                {item.description || translate("structure.noDescription")}
                            </span>
                            {isDepartment && agentCount && (
                                <span className="poph-mono-muted">
                                    {textAgents(agentCount(item.id), translate)}
                                </span>
                            )}
                        </div>
                        <div className="poph-structure__actions">
                            <span className={`poph-status ${classStatus(item.status)}`}>
                                <span className="poph-status__point" aria-hidden="true" />
                                {labelStatus(item.status, translate)}
                            </span>
                            <SofaButton
                                variant="subtle"
                                size="sm"
                                aria-label={
                                    isDepartment
                                        ? translate("structure.editDepartmentAria", { name: item.name })
                                        : translate("structure.editCategoryAria", { name: item.name })
                                }
                                onClick={() => setEdit({ opened: true, registro: item })}
                            >
                                {translate("structure.edit")}
                            </SofaButton>
                            <SofaButton
                                variant="subtle"
                                size="sm"
                                aria-label={
                                    isDepartment
                                        ? translate("structure.deleteDepartmentAria", { name: item.name })
                                        : translate("structure.deleteCategoryAria", { name: item.name })
                                }
                                onClick={() =>
                                    isDepartment
                                        ? setToDelete(item as Departamento)
                                        : setCategoryToDelete(item as Category)
                                }
                            >
                                {translate("structure.delete")}
                            </SofaButton>
                        </div>
                    </li>
                ))}
            </ul>
        );
    }

    return (
        <section className="poph-section poph-structure__section" aria-labelledby={titleId}>
            <div className="poph-structure__header">
                <div className="poph-structure__title-block">
                    <h2 id={titleId} className="poph-section__title">
                        {title}
                    </h2>
                    <p className="poph-structure__help">
                        {isDepartment
                            ? translate("structure.departmentsHelp")
                            : translate("structure.categoriesHelp")}
                    </p>
                </div>
                <div className="poph-structure__actions-section">
                    <SofaButton
                        variant="subtle"
                        size="sm"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => void load()}
                        loading={estado.loading && estado.items.length > 0}
                    >
                        {translate("structure.refresh")}
                    </SofaButton>
                    {buttonNew}
                </div>
            </div>

            {content}

            <ModalStructure
                tipo={tipo}
                org={org}
                opened={edit.opened}
                registro={edit.registro}
                onClose={() => setEdit({ opened: false, registro: null })}
                onSave={onSaveRecord}
            />

            {isDepartment && (
                <DialogDeleteDepartment
                    opened={toDelete !== null}
                    onClose={() => setToDelete(null)}
                    org={org}
                    departamento={toDelete}
                    onDelete={() => {
                        setToDelete(null);
                        onChange?.();
                        void load();
                    }}
                />
            )}

            {!isDepartment && (
                <DialogDeleteCategory
                    opened={categoryDelete !== null}
                    onClose={() => setCategoryToDelete(null)}
                    org={org}
                    category={categoryDelete}
                    onDelete={() => {
                        setCategoryToDelete(null);
                        onChange?.();
                        void load();
                    }}
                />
            )}
        </section>
    );
}

export function Structure() {
    const { translate } = useI18n();
    const { orgId = "" } = useParams();
    const [nameOrg, setOrgName] = useState<string | null>(null);
    const [agents, setAgents] = useState<AgentSummary[] | null>(null);

    const loadAgents = useCallback(async () => {
        try {
            setAgents(await listAgents(orgId));
        } catch {
            setAgents(null);
        }
    }, [orgId]);

    useEffect(() => {
        setOrgName(null);
        getOrg(orgId)
            .then((org) => setOrgName(org.name))
            .catch(() => setOrgName(null));
        void loadAgents();
    }, [orgId, loadAgents]);

    const count = (id: string): number | null =>
        agents === null ? null : agentsUsing(agents, id).length;

    return (
        <div className="poph-page poph-structure">
            <div className="poph-page__top">
                <div>
                    <p className="poph-structure__eyebrow">
                        {nameOrg
                            ? translate("structure.eyebrowOrg", { name: nameOrg })
                            : translate("structure.eyebrow")}
                    </p>
                    <h1 className="poph-page__title">{translate("structure.pageTitle")}</h1>
                    <p className="poph-page__description">
                        {translate("structure.pageDescription")}
                    </p>
                </div>
                <div className="poph-page__actions">
                    <SofaButton
                        variant="secondary"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => void loadAgents()}
                    >
                        {translate("structure.refreshAgents")}
                    </SofaButton>
                </div>
            </div>

            <SectionStructure
                tipo="departamento"
                org={orgId}
                agentCount={count}
                onChange={() => void loadAgents()}
            />
            <SectionStructure tipo="categoria" org={orgId} />
        </div>
    );
}
