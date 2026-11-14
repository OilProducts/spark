import { createContext, useContext, useLayoutEffect, useRef, type MutableRefObject, type PropsWithChildren } from 'react'
import type { Dispatch, SetStateAction } from 'react'
import type { Connection, Edge, Node } from '@xyflow/react'
import type { CanonicalFlowDefinition } from '@/lib/canonicalFlowModel'
import type { FlowDefinitionMetadata } from '@/store'

type EditorGraphBridge = {
  getNodes: () => Node[]
  setNodes: Dispatch<SetStateAction<Node[]>>
  getEdges: () => Edge[]
  setEdges: Dispatch<SetStateAction<Edge[]>>
  /** The loaded flow definition, which carries what the canvas does not edit. */
  getFlow: () => CanonicalFlowDefinition | null
  /**
   * Adds a route between two steps, as dragging one on the canvas does; `data` sets its fields.
   * Given a route's own ID, it replaces that route where it stands, such as with a new target.
   */
  connect: (params: (Connection & { data?: Record<string, unknown> }) | Edge) => void
  /**
   * Called before an inspector edit with the steps and routes it changes, or the flow settings, as
   * they are, so Undo can set them back; `key` names the field, and edits to it in a row are one,
   * unless null.
   */
  recordFieldEdit: (key: string | null, previous: { nodes?: Node[]; edges?: Edge[]; flowMetadata?: FlowDefinitionMetadata }) => void
}

const EditorGraphBridgeContext = createContext<MutableRefObject<EditorGraphBridge | null> | null>(null)

export function EditorGraphBridgeProvider({ children }: PropsWithChildren) {
  const bridgeRef = useRef<EditorGraphBridge | null>(null)
  return (
    <EditorGraphBridgeContext.Provider value={bridgeRef}>
      {children}
    </EditorGraphBridgeContext.Provider>
  )
}

export function useRegisterEditorGraphBridge(bridge: EditorGraphBridge) {
  const bridgeRef = useContext(EditorGraphBridgeContext)
  useLayoutEffect(() => {
    if (!bridgeRef) {
      return
    }
    bridgeRef.current = bridge
    return () => {
      if (bridgeRef.current === bridge) {
        bridgeRef.current = null
      }
    }
  }, [bridge, bridgeRef])
}

export function useEditorGraphBridgeRef() {
  return useContext(EditorGraphBridgeContext)
}
