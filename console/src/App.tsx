
import { useEffect } from "react";
import { Navigate, Outlet, Route, Routes, useLocation, useParams } from "react-router-dom";
import { GuardChangesProvider } from "./components/GuardChanges";
import { Shell } from "./components/Shell";
import { resolveDefaultOrgPath, useSession, writeLastOrgId } from "./session/Session";
import { AgentKnowledge } from "./telas/AgenteConhecimento";
import { AgentFiles } from "./telas/AgenteArquivos";
import { AgentChatTest } from "./telas/AgenteChatTeste";
import { AgentInstructions } from "./telas/AgenteInstrucoes";
import { AgentModelJev } from "./telas/AgenteModeloJev";
import { AgentIntegrations } from "./telas/AgenteIntegracoes";
import { AgentRouting } from "./telas/AgenteRoteamento";
import { Agents } from "./telas/Agentes";
import { Login } from "./telas/Login";
import { Structure } from "./telas/Estrutura";
import { OrgPanel } from "./telas/OrgPainel";
import { Orgs } from "./telas/Orgs";
import { Products } from "./telas/Produtos";
import { Operation } from "./telas/Operacao";
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
        <GuardChangesProvider>
            <RememberLastOrg />
            <Shell>
                <Outlet />
            </Shell>
        </GuardChangesProvider>
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
                <Route path="/orgs/:orgId" element={<OrgPanel />} />
                <Route path="/orgs/:orgId/structure" element={<Structure />} />
                <Route path="/orgs/:orgId/products" element={<Products />} />
                <Route path="/orgs/:orgId/operacao" element={<Operation />} />
                <Route path="/orgs/:orgId/matrix" element={<Matrix />} />
                <Route path="/orgs/:orgId/agents" element={<Agents />} />
                <Route path="/orgs/:orgId/agents/:agentId" element={<AgentWithoutTab />} />
                <Route
                    path="/orgs/:orgId/agents/:agentId/instructions"
                    element={<AgentInstructions />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/knowledge-base"
                    element={<AgentKnowledge />}
                />
                <Route path="/orgs/:orgId/agents/:agentId/files" element={<AgentFiles />} />
                <Route
                    path="/orgs/:orgId/agents/:agentId/routing"
                    element={<AgentRouting />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/integrations"
                    element={<AgentIntegrations />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/model-jev"
                    element={<AgentModelJev />}
                />
                <Route
                    path="/orgs/:orgId/agents/:agentId/test-chat"
                    element={<AgentChatTest />}
                />
                <Route path="*" element={<SessionHome />} />
            </Route>
        </Routes>
    );
}
