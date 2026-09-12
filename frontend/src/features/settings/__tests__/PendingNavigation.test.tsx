import { act, cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { fetchRuntimeSettings, saveRuntimeSettings } from '@/lib/api/settingsApi'
import { RuntimeSettingsEditor } from '@/features/settings/RuntimeSettingsEditor'
vi.mock('@/lib/api/settingsApi', () => ({ fetchRuntimeSettings: vi.fn(), saveRuntimeSettings: vi.fn() }))
afterEach(() => {cleanup(); vi.resetAllMocks()})
it(' navigation must stay blocked while a runtime save is pending', async () => {
 const view={scope:'workspace',revision:'one',restart_fields:['flows_dir'],stored:{flows_dir:'/saved',runs_dir:null,ui_dir:null,project_roots:[]},effective:{flows_dir:'/saved',runs_dir:null,ui_dir:null,project_roots:[]}}
 vi.mocked(fetchRuntimeSettings).mockResolvedValue(view)
 vi.mocked(saveRuntimeSettings).mockImplementation(()=>new Promise(()=>{}))
 render(<DialogProvider><RuntimeSettingsEditor/></DialogProvider>)
 const user=userEvent.setup();const field=await screen.findByLabelText('Flows directory')
 await user.clear(field);await user.type(field,'/draft')
 await user.click(screen.getByRole('button',{name:'Save runtime settings'}))
 expect(screen.getByRole('button',{name:'Save runtime settings'})).toBeDisabled()
 const proceed=vi.fn()
 act(()=>window.dispatchEvent(new CustomEvent('spark:before-navigation',{cancelable:true,detail:{proceed}})))
 const leave=screen.queryByRole('button',{name:'Discard and leave'})
 if(leave) await user.click(leave)
 await act(async()=>{})
 expect(proceed,'A still-running save must not permit Discard-and-leave navigation').not.toHaveBeenCalled()
})
