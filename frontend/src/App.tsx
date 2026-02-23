import { Navbar } from "./components/Navbar"
import { Sidebar } from "./components/Sidebar"
import { Terminal } from "./components/Terminal"
import { Editor } from "./components/Editor"
import { RunStream } from "./components/RunStream"
import { ReactFlowProvider } from "@xyflow/react"

function App() {
  return (
    <ReactFlowProvider>
      <RunStream />
      <div className="h-screen flex flex-col antialiased bg-background text-foreground">
        <Navbar />
        <div className="flex-1 flex overflow-hidden">
          <Sidebar />
          <main className="flex-1 relative flex flex-col overflow-hidden bg-muted/10">
            <div className="flex-1 w-full h-full bg-background/50">
              <Editor />
            </div>
            <Terminal />
          </main>
        </div>
      </div>
    </ReactFlowProvider>
  )
}

export default App
