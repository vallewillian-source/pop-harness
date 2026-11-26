
import { useEffect } from "react";
import { Navigate, Outlet, Route, Routes, useLocation, useParams } from "react-router-dom";
import { GuardaAlteracoesProvider } from "./componentes/GuardaAlteracoes";
import { Shell } from "./componentes/Shell";
import { resolveDefaultOrgPath, useSession, writeLastOrgId } from "./session/Session";
import { AgenteConhecimento } from "./telas/AgenteConhecimento";
import { AgenteArquivos } from "./telas/AgenteArquivos";
import { AgenteChatTeste } from "./telas/AgenteChatTeste";
import { AgenteInstrucoes } from "./telas/AgenteInstrucoes";
import { AgenteModeloJev } from "./telas/AgenteModeloJev";
import { AgenteIntegracoes } from "./telas/AgenteIntegracoes";
import { AgenteRoteamento } from "./telas/AgenteRoteamento";
import { Agentes } from "./telas/Agentes";
import { Login } from "./telas/Login";
import { Estrutura } from "./telas/Estrutura";
import { OrgPainel } from "./telas/OrgPainel";
import { Orgs } from "./telas/Orgs";
import { Produtos } from "./telas/Produtos";
import { Operacao } from "./telas/Operacao";
import { Matrix } from "./telas/Matrix";

function RememberLastOrg() {
    const { "sessao": session } = useSession();
    const { pathname } = useLocation();
    useEffect(() => {
        const [root, orgId] = pathname.split("/").filter(Boolean);
        if (root !== "orgs" || !orgId) return;
        if (session?.orgs.some((org) => org.id === orgId)) writeLastOrgId(orgId);
    }, [pathname, session]);
    return null;
}

function Protegido() {
    const { "sessao": session } = useSession();
    if (!session) {
        return <Navigate to="/login" replace />;
    }
    return (
        <GuardaAlteracoesProvider>
            <RememberLastOrg />
            <Shell>
                <Outlet />
            </Shell>
        </GuardaAlteracoesProvider>
    );
}

function SessionHome() {
    const { "sessao": session } = useSession();
    return <Navigate to={resolveDefaultOrgPath(session?.orgs ?? [])} replace />;
}

function LoginOrSignIn() {
    const { "sessao": session } = useSession();
    return session ? <Navigate to={resolveDefaultOrgPath(session.orgs)} replace /> : <Login />;
}

function AgentWithoutTab() {
    const { orgId = "", agentId = "" } = useParams();
    return <Navigate to={`/orgs/${orgId}/agents/${agentId}/instructions`} replace />;
}

export function App() {
    return (
        <Routes>
            <Route path="/login" element={<LoginOrSignIn />} />
            <Route element={<Protegido />}>
                <Route path="/" element={<SessionHome />} />
                <Route path="/orgs" element={<Orgs />} />
                <Route path="/orgs/:orgId" element={<OrgPainel />} />
                <Route path="/orgs/:orgId/structure" element={<Estrutura />} />
                <Route path="/orgs/:orgId/products" element={<Produtos />} />
                <Route path="/orgs/:orgId/operacao" element={<Operacao />} />
                <Route path="/orgs/:orgId/matrix" element={<Matrix />} />
                <Route path="/orgs/:orgId/agents" element={<Agentes />} />
                <Route path="/orgs/:orgId/agents/:agentId" element={<AgentWithoutTab />} />
                <Route
                    path="/orgs/:orgId/agents/:agentId/instructions"
                    element={<AgenteInstrucoes />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/knowledge-base"
                    element={<AgenteConhecimento />}
                />
                <Route path="/orgs/:orgId/agents/:agentId/files" element={<AgenteArquivos />} />
                <Route
                    path="/orgs/:orgId/agents/:agentId/routing"
                    element={<AgenteRoteamento />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/integrations"
                    element={<AgenteIntegracoes />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/model-jev"
                    element={<AgenteModeloJev />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/test-chat"
                    element={<AgenteChatTeste />}
                />
                <Route path="*" element={<SessionHome />} />
            </Route>
        </Routes>
    );
}
