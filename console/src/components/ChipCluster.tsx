
import type { SofaIconProps } from "@sofacoding/sofa-ui";
import { useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";

export type SituationCluster = "ok" | "degradada" | "sem-conexao" | "verificando";

const LABELS: Record<SituationCluster, TranslationKey> = {
    ok: "orgs.clusterOk",
    degradada: "orgs.clusterDegraded",
    "sem-conexao": "orgs.clusterOffline",
    verificando: "orgs.clusterChecking",
};

const CLASS_NAMES: Record<SituationCluster, string> = {
    ok: "poph-rail__cluster-point",
    degradada: "poph-rail__cluster-point poph-rail__cluster-point--alert",
    "sem-conexao": "poph-rail__cluster-point poph-rail__cluster-point--alert",
    verificando: "poph-rail__cluster-point poph-rail__cluster-point--neutral",
};

function IconServer(props: SofaIconProps) {
    return (
        <svg
            viewBox="0 0 24 24"
            width="14"
            height="14"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.25"
            strokeLinecap="round"
            strokeLinejoin="round"
            {...props}
        >
            <path d="M4 5h16a1 1 0 0 1 1 1v3a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1z" />
            <path d="M4 14h16a1 1 0 0 1 1 1v3a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-3a1 1 0 0 1 1-1z" />
            <path d="M6 7.5h.01" />
            <path d="M6 16.5h.01" />
        </svg>
    );
}

export function ChipDoCluster({
    situacao,
    compact = false,
}: {
    situacao: SituationCluster;
    compact?: boolean;
}) {
    const { translate: t } = useI18n();
    const label = t("orgs.clusterLabel", { status: t(LABELS[situacao]) });
    return (
        <div
            className={
                compact
                    ? "poph-rail__cluster poph-rail__cluster--compact"
                    : "poph-rail__cluster"
            }
            title={label}
        >
            <IconServer className="poph-rail__cluster-icon" aria-hidden="true" />
            <span className="poph-rail__cluster-name">POP Harness Cluster</span>
            <span className={CLASS_NAMES[situacao]} aria-hidden="true" />
            <span className="poph-rail__only-reader">{label}</span>
        </div>
    );
}
