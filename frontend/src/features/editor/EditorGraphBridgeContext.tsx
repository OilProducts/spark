import { createContext, useContext, useLayoutEffect, useRef, type MutableRefObject, type PropsWithChildren } from 'react'
import type { Dispatch, SetStateAction } from 'react'
import type { Edge, Node } from '@xyflow/react'
import type { CanonicalFlowDefinition } from '@/lib/canonicalFlowModel'

type EditorGraphBridge = {
  getNodes: () => Node[]
  setNodes: Dispatch<SetStateAction<Node[]>>
  getEdges: () => Edge[]
  setEdges: Dispatch<SetStateAction<Edge[]>>
  /** The loaded flow definition, which carries what the canvas does not edit. */
  getFlow: () => CanonicalFlowDefinition | null
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
