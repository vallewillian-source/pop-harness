
import { useEffect, useState } from "react";
import { SofaSkeleton } from "@sofacoding/sofa-ui";
import { listItems, deleteGroup, type KnowledgeGroup, type KnowledgeItem } from "../api/knowledge";
import { useI18n } from "../i18n";
import { DialogExclusion } from "../components/DialogExclusion";

export type DialogDeleteGroupProps = {
    opened: boolean;
    onClose: () => void;
    org: string;
    group: KnowledgeGroup | null;
    namesAgents: Record<string, string>;
    onDelete: () => void;
};

export function DialogDeleteGroup({
    opened: isOpen,
    onClose,
    org,
    group,
    namesAgents,
    onDelete,
}: DialogDeleteGroupProps) {
    const { translate: t } = useI18n();
    const [loading, setLoading] = useState(false);
    const [items, setItems] = useState<KnowledgeItem[] | null>(null);
    const [failureCount, setFailureCount] = useState(false);

    useEffect(() => {
        if (!isOpen || !group) return;
        let cancelled = false;
        setLoading(true);
        setItems(null);
        setFailureCount(false);
        listItems(org, group.id)
            .then((list) => {
                if (!cancelled) setItems(list);
            })
            .catch(() => {
                if (!cancelled) setFailureCount(true);
            })
            .finally(() => {
                if (!cancelled) setLoading(false);
            });
        return () => {
            cancelled = true;
        };
    }, [isOpen, group, org]);

    if (!group) return null;

    const agents = group.agents ?? [];

    function consequencia() {
        if (loading) {
            return <SofaSkeleton lines={2} />;
        }
        if (failureCount || items === null) {
            return (
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {t("knowledge.deleteGroupCannotCount")}
                </p>
            );
        }
        const titles =
            items
                .slice(0, 6)
                .map((item) => item.title)
                .join(", ") +
            (items.length > 6
                ? t("knowledge.deleteGroupMoreItems", { count: items.length - 6 })
                : "");
        return (
            <div className="poph-stack" style={{ gap: "var(--pop-spacing-sm, 8px)" }}>
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {items.length === 0
                        ? t("knowledge.deleteGroupEmpty")
                        : t(
                              items.length === 1
                                  ? "knowledge.deleteGroupItemsOne"
                                  : "knowledge.deleteGroupItemsMany",
                              { count: items.length, titles },
                          )}
                </p>
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {agents.length === 0
                        ? t("knowledge.deleteGroupNoAgents")
                        : t(
                              agents.length === 1
                                  ? "knowledge.deleteGroupAgentsOne"
                                  : "knowledge.deleteGroupAgentsMany",
                              {
                                  count: agents.length,
                                  names: agents
                                      .map((id) => namesAgents[id] ?? id)
                                      .join(", "),
                              },
                          )}
                </p>
            </div>
        );
    }

    return (
        <DialogExclusion
            opened={isOpen}
            onClose={onClose}
            title={t("knowledge.deleteGroupTitle")}
            descricao={t("knowledge.deleteGroupDescription", { name: group.name })}
            resource={group.name}
            labelResource={t("knowledge.deleteGroupResourceLabel")}
            labelConfirm={t("knowledge.deleteGroupConfirm")}
            consequencia={consequencia()}
            allowForce={false}
            onConfirm={async () => {
                await deleteGroup(org, group.id);
            }}
            onDelete={onDelete}
        />
    );
}
