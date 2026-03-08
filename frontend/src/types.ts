export type ViewMode = 'home' | 'projects' | 'editor' | 'execution' | 'settings' | 'runs';

export interface SparkspawnState {
    viewMode: ViewMode;
    setViewMode: (mode: ViewMode) => void;
    activeFlow: string | null;
    setActiveFlow: (flow: string | null) => void;
    selectedNodeId: string | null;
    setSelectedNodeId: (id: string | null) => void;
}
