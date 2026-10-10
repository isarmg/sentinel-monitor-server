import {readFile} from 'node:fs/promises';
import {registerHooks} from 'node:module';
import {fileURLToPath} from 'node:url';
import {transformWithOxc} from 'vite';
const entry=new URL('../../src/main.tsx',import.meta.url);
const original=await readFile(entry,'utf8');
const source=original+'\nexport { Console, RecordingsView, CameraDrawer, LiveVideo, localDateInput, formatDate };';
const {code}=await transformWithOxc(source,fileURLToPath(entry),{jsx:{runtime:'automatic'}});
const host=new URL('./hook-host.mjs',import.meta.url).href;
const stubs={
 '@xcss/web/web-fonts':'export function startAfterFonts() {}',
 '@xcss/web/admin-ui/date-range':'export const DateRangeField="date-range"',
 '@xcss/web/admin-ui/i18n':'export function t(zh,en,args=[]){return en.replace(/\\{(\\d+)\\}/g,(_,n)=>args[n])}; export const getLocale=()=>"en";',
 'react-dom/client':'export const createRoot=()=>({render(){}});',
 '@xcss/web/admin-shell':'export const createXcssAdminApplication=()=>"root", errorRequestId=()=>undefined, useAdminApplication=()=>({notify(){}}), InstancePageNavigation="navigation", InstanceHeaderActions="actions", AccountPage="account";',
 '@xcss/web/admin-ui':'export const Button="button", ConfirmDangerDialog="confirm", Dialog="dialog", EmptyState="empty", ErrorState="error", FormField="field", LoadingState="loading", Select="select", Table="table", TextField="input";',
 './display-labels':'export const displayLabel = x=>x;',
 './camera-status':'export const effectiveStatus = x=>x.status;',
 './whep':'export class WhepPlayer {}',
 './media-url':'export const isSameOriginMediaUrl=()=>true, requireSameOriginMediaUrl=x=>x;',
 './snapshot-refresh':'export const createSnapshotRefresh=()=>async()=>{};',
 './api':'export const administratorApi={},apiPath=x=>`/api/v1${x}`, request=(...args)=>globalThis.probeRequest(...args),requestLog=(...args)=>globalThis.probeRequest(...args); export const isAuditLogPage=()=>true,isLogCalendar=()=>true,isCameras=()=>true,isEventLogPage=()=>true,isOperation=()=>true,isOperationLogPage=()=>true,isRecordingSpans=()=>true,isStreamTicket=()=>true,isXcosClient=()=>true,isXcosClients=()=>true,isSystemStatus=()=>true,isUndefined=()=>true,isDeviceCommandReceipt=()=>true;'
};
export async function loadOriginalMain(){
 globalThis.window??={location:{href:'https://camera.example/',hash:'#details'},setTimeout,clearTimeout,setInterval,clearInterval,addEventListener(){},removeEventListener(){}};
 globalThis.document??={getElementById:()=>({}),addEventListener(){},removeEventListener(){},hidden:false};
 const hooks=registerHooks({
  resolve(specifier,context,next){
   if(specifier==='react'||specifier==='react/jsx-runtime')return{url:host,shortCircuit:true};
   const stub=specifier.endsWith('.css')?'':stubs[specifier];
   if(stub!==undefined)return{url:`data:text/javascript,${encodeURIComponent(stub)}`,shortCircuit:true};
   return next(specifier,context);
  },
  load(url,context,next){if(url===entry.href)return{format:'module',source:code,shortCircuit:true};return next(url,context);}
 });
 try{return await import(entry.href);}finally{hooks.deregister();}
}
