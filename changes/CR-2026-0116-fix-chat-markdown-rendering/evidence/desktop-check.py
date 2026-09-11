import subprocess,time,json,pathlib
out=pathlib.Path('changes/CR-2026-0116-fix-chat-markdown-rendering/evidence')
def osa(s):
 r=subprocess.run(['osascript','-e',s],capture_output=True,text=True)
 if r.returncode: raise RuntimeError(r.stderr)
 return r.stdout.strip()
p=subprocess.Popen(['/tmp/spark-cr0116-desktop'])
time.sleep(3)
log=[]
for mode in ['mouse','keyboard']:
 for n,scheme in [(1,'https'),(2,'http')]:
  setup='''tell application "System Events"
 tell process "spark-cr0116-desktop"
 set frontmost to true
 set links to {}
 set elements to entire contents of window 1
 repeat with e in elements
 if role of e is "AXLink" then set end of links to contents of e
 end repeat
 set targetLink to item %d of links
 ''' % n
  if mode=='mouse':
   action='set pos to position of targetLink\nset sz to size of targetLink\nclick at {(item 1 of pos) + (item 1 of sz) / 2, (item 2 of pos) + (item 2 of sz) / 2}'
  else:
   action='set value of attribute "AXFocused" of targetLink to true\nkey code 36'
  result=osa(setup+action+'\nend tell\nend tell')
  time.sleep(3)
  urls=osa('tell application "Google Chrome" to get URL of every tab of every window')
  log.append({'mode':mode,'scheme':scheme,'time':time.strftime('%Y-%m-%dT%H:%M:%S%z'),'browser_urls':urls})
  print(json.dumps(log[-1]),flush=True)
  (out/'desktop-actions.json').write_text(json.dumps(log,indent=2))
osa('tell application "System Events" to tell process "spark-cr0116-desktop" to set frontmost to true')
subprocess.run(['screencapture','-x','-R116,61,1280,840',str(out/'desktop-renderer.png')])
print(osa('tell application "System Events" to tell process "spark-cr0116-desktop" to get entire contents of window 1'))
time.sleep(10)
