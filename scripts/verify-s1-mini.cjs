// CPU-only evaluation of S1-mini by Superwhisper. Requires an existing local model and llama.cpp runtime; downloads nothing.
const fs=require('fs'),path=require('path'),net=require('net'),crypto=require('crypto'),{spawn}=require('child_process');
const dir=process.env.TYPR_S1_DIR||path.join(process.env.APPDATA,'com.typr.app','local-cleanup');
const outputDir=process.env.TYPR_S1_OUTPUT||path.join(process.cwd(),'.impeccable','review');fs.mkdirSync(outputDir,{recursive:true});
const model=path.join(dir,'s1-mini-q4_k_m.gguf');
if(crypto.createHash('sha256').update(fs.readFileSync(model)).digest('hex')!=='3b41ebe2502cbd03e811d5d16b022f5ab551eda58d62597d152f89535003c634')throw Error('Model checksum mismatch');
const system='You are a text normalizer for speech-to-text transcripts. The input begins with a control line specifying the styling, structure, and context settings; clean the transcript to match those settings and output only the cleaned text.';
const cases=[
 {id:'punctuation',input:'review the invoice schedule the meeting keep the final attachment',required:['review the invoice','schedule the meeting','keep the final attachment']},
 {id:'negation',input:'do not delete the project the feature is not enabled keep every final word',required:['not delete','not enabled','every final word']},
 {id:'technical-words',input:'the picture shows our team taking a shot add a quick overlay button including robust decoding algorithms',required:['picture','team taking a shot','quick overlay button','robust decoding algorithms']},
 {id:'names',input:'open the glint app and the tauri repository ask synara to review herdr with pi agent',required:['glint','tauri','synara','herdr','pi agent']},
 {id:'literal',input:'keep src/main.rs and https://example.com/a?b=1 unchanged do not rename user_id',required:['src/main.rs','https://example.com/a?b=1','not rename','user_id']},
 {id:'short-question',input:'can you check whether the release is ready',required:['can you check whether the release is ready']},
 {id:'correction',input:'send the invoice on friday no wait thursday',required:['thursday'],absent:['friday']},
 {id:'email',input:'please send it to hello at example dot com',required:['hello@example.com']},
 {id:'number',input:'the total is twenty three dollars and fifty cents',required:['23.50']},
 {id:'filler-only',input:'um uh',empty:true},
 {id:'list',input:'first review the invoice second schedule the meeting third attach the report',structure:'lists',required:['review the invoice','schedule the meeting','attach the report'],bullets:true},
 {id:'long-ending',input:'i want you to review the application and check whether the settings are saved correctly make sure the dictionary only suggests terms after a real correction and never replaces ordinary words automatically do not remove any qualifiers keep the last sentence exactly as spoken including robust decoding algorithms',required:['only suggests terms','real correction','never replaces ordinary words automatically','not remove any qualifiers','including robust decoding algorithms']},
];
(async()=>{
 const reservation=net.createServer();await new Promise(r=>reservation.listen(0,'127.0.0.1',r));const port=reservation.address().port;await new Promise(r=>reservation.close(r));
 const log=fs.openSync(path.join(outputDir,'s1-server.log'),'w');
 const child=spawn(path.join(dir,'runtime','llama-server.exe'),['--model',model,'--host','127.0.0.1','--port',String(port),'--ctx-size','4096','--parallel','1','--n-gpu-layers','0','--threads','4','--threads-batch','4','--jinja','--chat-template-kwargs','{"enable_thinking":false}','--temp','0','--no-webui'],{windowsHide:true,stdio:['ignore',log,log]});
 try {
  const base=`http://127.0.0.1:${port}`;
  let ready=false;for(let i=0;i<240;i++){try{const r=await fetch(base+'/health',{signal:AbortSignal.timeout(1000)});if(r.ok){ready=true;break}}catch{}await new Promise(r=>setTimeout(r,250));if(child.exitCode!==null)throw Error('Server exited');}
  if(!ready)throw Error('Server not ready');const results=[];
  for(const test of cases){
   const start=performance.now();
   const response=await fetch(base+'/v1/chat/completions',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({messages:[{role:'system',content:system},{role:'user',content:`[Styling: semi-formal] [Structure: ${test.structure||'prose'}] [Context: general]\n${test.input}`}],temperature:0,max_tokens:Math.max(96,Math.ceil(test.input.length/3*1.4)+32),chat_template_kwargs:{enable_thinking:false}}),signal:AbortSignal.timeout(30000)});
   if(!response.ok)throw Error('Inference HTTP '+response.status);const json=await response.json();const output=json.choices?.[0]?.message?.content??'';const normalized=output.toLowerCase();
   const failures=[];for(const word of test.required||[])if(!normalized.includes(word))failures.push('missing '+word);for(const word of test.absent||[])if(normalized.includes(word))failures.push('retained '+word);
   if(test.empty&&output.trim())failures.push('not empty');if(test.bullets&&!/^(?:- |\d+[.)] )/m.test(output))failures.push('no list structure');if(json.choices?.[0]?.finish_reason==='length')failures.push('truncated');
   const row={id:test.id,input:test.input,output,ms:Math.round(performance.now()-start),failures};results.push(row);console.log(JSON.stringify(row));
  }
  fs.writeFileSync(path.join(outputDir,'s1-results.json'),JSON.stringify(results,null,2));console.log('Passed '+results.filter(r=>!r.failures.length).length+'/'+results.length);
 }finally{if(child.exitCode===null&&child.signalCode===null)await new Promise(r=>{child.once('exit',r);child.kill();});fs.closeSync(log);}
})().catch(e=>{console.error(e);process.exitCode=1});
