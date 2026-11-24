/**
 * Menu lateral do Console (PRD do re-layout §5.2).
 *
 * Desde a RL-03: seletor de organização, três grupos (Tenants e recursos, Runtime e motor, Sistema)
 * lidos de `navegacao.ts`, contador em Organizações e em Agentes (esse último vem de fora, pela
 * casca) e rodapé com a versão da instância e o botão de atalhos.
 *
 * CF-02 (desenho `design/pop-harness-console-v2.figcraft`): largura de 240 px e modo foco de 64 px
 * (prancha 04), com o botão que alterna os dois estados.
 *
 * CF-06: o topo do rail depende da tela, como no desenho. Na lista (`/orgs`, prancha 01) é o chip de
 * identidade do cluster (`ChipDoCluster`); com uma organização aberta (pranchas 02 e 05) é o seletor
 * de organização (`SeletorDeOrganizacao`, o `OrgWorkspaceSwitcherCombobox`). Nunca os dois.
 * CF-07: na visão do agente (`/orgs/:id/agents/:agentId/...`, prancha 03) o topo é o link
 * "← Agentes da Org" mais o combobox do agente (`SeletorDeAgente`), no lugar do seletor de
 * organização.
 * O item ativo usa o degrau tonal da SofaUI, sem a barra âmbar que existia aqui: o desenho não tem
 * borda lateral colorida. No modo foco o rótulo de cada item continua no DOM, escondido por recorte,
 * e o `title` devolve o nome ao mouse. A casca põe este menu abaixo da barra superior.
 */

import { Fragment, useRef, type ReactNode } from "react";
import { NavLink, useLocation } from "react-router-dom";
import {
    IconChevronLeft,
    IconChevronRight,
    SofaButton,
    SofaRailItem,
    SofaSidebarRail,
} from "@sofacoding/sofa-ui";
import { useSessao } from "../sessao/Sessao";
import {
    GRUPOS_DE_NAVEGACAO,
    ITENS_DE_NAVEGACAO,
    type ContextoDeNavegacao,
    type ItemDeNavegacao,
} from "./navegacao";
import { ChipDoCluster, type SituacaoDoCluster } from "./ChipDoCluster";
import { SeletorDeOrganizacao } from "./SeletorDeOrganizacao";
import { SeletorDeAgente, VoltarParaAgentes } from "./SeletorDeAgente";
import { useSaude } from "./saude";
import "../estilos/navegacao.css";

/**
 * Motivo que aparece ao passar o mouse num item ativo que está sem contexto na URL. Não é texto
 * visível na linha: vai no `title` do wrapper.
 */
function motivoDoBloqueio(item: ItemDeNavegacao, contexto: ContextoDeNavegacao): string | undefined {
    if (item.exigeOrg && !contexto.orgId) return "Requer uma organização aberta";
    if (item.exigeAgente && !contexto.agentId) return "Requer um agente aberto";
    return undefined;
}

/**
 * Um item do menu. Sem rota vira botão desabilitado; com rota, vira `NavLink` para a rota
 * resolvida. "em breve" aparece só nos itens de estado `emBreve`. Um item ativo sem contexto
 * (organização ou agente ausente na URL) fica desabilitado sem etiqueta visível: o motivo vai no
 * `title` de um wrapper, porque o `SofaRailItem` não repassa `title` nem `aria-*`.
 */
export function ItenDeNavegacao({
    item,
    contexto,
    compacta = false,
}: {
    item: ItemDeNavegacao;
    contexto: ContextoDeNavegacao;
    compacta?: boolean;
}) {
    const icone: ReactNode = <item.icone aria-hidden="true" />;
    const contagem = item.contador?.(contexto) ?? null;
    const aside: ReactNode =
        contagem === null ? undefined : <span className="poph-rail__contador">{contagem}</span>;
    const rota = item.rota(contexto);
    if (item.estado === "emBreve" || rota === null) {
        const emBreve = item.estado === "emBreve";
        const botao = (
            <SofaRailItem
                className="poph-rail__item"
                label={item.rotulo}
                media={icone}
                aside={aside}
                description={emBreve ? "em breve" : undefined}
                compact={compacta}
                disabled
            />
        );
        const motivo = emBreve ? undefined : motivoDoBloqueio(item, contexto);
        if (!motivo) return botao;
        return (
            <div className="poph-rail__bloqueado" title={compacta ? item.rotulo : motivo}>
                {botao}
            </div>
        );
    }
    return (
        <NavLink
            to={rota}
            end={item.exato === true}
            title={compacta ? item.rotulo : undefined}
            style={{ color: "inherit", textDecoration: "none" }}
        >
            {({ isActive }) => (
                <SofaRailItem
                    className="poph-rail__item"
                    label={item.rotulo}
                    media={icone}
                    aside={aside}
                    compact={compacta}
                    active={isActive}
                />
            )}
        </NavLink>
    );
}

export function Rail({
    aoAbrirAtalhos,
    totalDeAgentes = null,
    compacta = false,
    aoAlternarCompacta = () => {},
}: {
    aoAbrirAtalhos: () => void;
    /** Agentes da organização aberta (contador do item Agentes); `null` = sem contador. */
    totalDeAgentes?: number | null;
    /** Estado visual do rail: expandido (240px) ou modo foco (64px). */
    compacta?: boolean;
    aoAlternarCompacta?: () => void;
}) {
    const { sessao } = useSessao();
    const { saude: dados, semConexao, degradada } = useSaude();
    const { pathname } = useLocation();
    // O botão que alterna o rail é remontado no outro extremo a cada troca de estado; sem devolver o
    // foco, quem navega por teclado cai no começo da página depois de recolher ou expandir. O foco é
    // movido no próprio acionamento (e não num efeito): em StrictMode o efeito rodaria de novo na
    // montagem e a tela abriria com o foco no rodapé.
    const botaoDoTopo = useRef<HTMLButtonElement>(null);
    const botaoDoRodape = useRef<HTMLButtonElement>(null);
    const alternar = () => {
        aoAlternarCompacta();
        requestAnimationFrame(() => {
            (compacta ? botaoDoRodape.current : botaoDoTopo.current)?.focus();
        });
    };
    // Organização aberta na URL (`/orgs/:id/...`): habilita os itens que dependem dela.
    const partesDaUrl = pathname.split("/").filter(Boolean);
    const orgAberta = partesDaUrl[0] === "orgs" ? (partesDaUrl[1] ?? "") : "";
    const agenteAberto = orgAberta && partesDaUrl[2] === "agents" ? (partesDaUrl[3] ?? "") : "";
    // Visão do agente (prancha 03): o topo do rail passa a ser o link de volta + o combobox do agente.
    const emRotaDeAgente = Boolean(orgAberta && agenteAberto);

    const orgs = sessao?.orgs ?? [];
    // Com token de org o seletor é informativo: mostra a própria org e não deixa trocar.
    const selecionada = sessao?.escopo === "org" ? (orgs[0]?.id ?? "") : orgAberta;
    const naListaDeOrgs = partesDaUrl.length === 1 && partesDaUrl[0] === "orgs";
    // Situação do ponto do chip do cluster: a mesma leitura de saúde que o rodapé já usa.
    const situacaoDoCluster: SituacaoDoCluster = semConexao
        ? "sem-conexao"
        : degradada
          ? "degradada"
          : dados
            ? "ok"
            : "verificando";
    const contexto: ContextoDeNavegacao = {
        orgId: orgAberta || null,
        agentId: agenteAberto || null,
        totalDeOrgs: sessao ? orgs.length : null,
        totalDeAgentes,
    };

    return (
        <SofaSidebarRail
            width={compacta ? 64 : 240}
            compact={compacta}
            label="Navegação do Console"
            footer={
                <div className="poph-rail__base">
                    {!compacta && (
                        <span className="poph-rail__versao">
                            POP Harness v{dados?.version ?? "—"} · Rust
                        </span>
                    )}
                    <SofaButton
                        variant="subtle"
                        size="sm"
                        className="poph-rail__atalhos"
                        aria-label="? atalhos"
                        title="Atalhos do teclado (?)"
                        onClick={aoAbrirAtalhos}
                    >
                        ?
                    </SofaButton>
                    {!compacta && (
                        <button
                            ref={botaoDoRodape}
                            type="button"
                            className="poph-rail__controle"
                            aria-label="Recolher menu lateral"
                            title="Recolher menu lateral"
                            onClick={alternar}
                        >
                            <IconChevronLeft aria-hidden="true" />
                        </button>
                    )}
                </div>
            }
        >
            {compacta && (
                <button
                    ref={botaoDoTopo}
                    type="button"
                    className="poph-rail__controle poph-rail__controle--topo"
                    aria-label="Expandir menu lateral"
                    title="Expandir menu lateral"
                    onClick={alternar}
                >
                    <IconChevronRight aria-hidden="true" />
                </button>
            )}
            {naListaDeOrgs ? (
                <ChipDoCluster situacao={situacaoDoCluster} compacta={compacta} />
            ) : emRotaDeAgente ? (
                <>
                    <VoltarParaAgentes orgId={orgAberta} compacta={compacta} />
                    <SeletorDeAgente orgId={orgAberta} agentId={agenteAberto} compacta={compacta} />
                </>
            ) : (
                <SeletorDeOrganizacao
                    orgs={orgs}
                    escopo={sessao?.escopo ?? "global"}
                    selecionada={selecionada}
                    naListaDeOrgs={naListaDeOrgs}
                    compacta={compacta}
                />
            )}
            {!compacta && <div className="poph-rail__separador" aria-hidden="true" />}

            {GRUPOS_DE_NAVEGACAO.map((grupo) => (
                <Fragment key={grupo}>
                    <div className="sofa-sidebar-rail__section-title">{grupo}</div>
                    {ITENS_DE_NAVEGACAO.filter((item) => item.grupo === grupo).map((item) => (
                        <ItenDeNavegacao
                            key={item.rotulo}
                            item={item}
                            contexto={contexto}
                            compacta={compacta}
                        />
                    ))}
                </Fragment>
            ))}
        </SofaSidebarRail>
    );
}
