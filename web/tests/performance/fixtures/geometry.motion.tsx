export const composition = { width: 640, height: 360, fps: 60, duration: 10 };
const CELLS=Array.from({length:128},(_,i)=>({id:`g${i}`,x:24+(i%16)*36,y:32+Math.floor(i/16)*32}));
export default function Geometry(ctx) { return <Scene style={{width:640,height:360,backgroundColor:"#101010"}}>
  {CELLS.map(c=><View key={c.id} style={{position:"absolute",left:c.x+Math.floor(ctx.progress*12),top:c.y,width:20,height:20,backgroundColor:"#8b7bff",opacity:0.25+ctx.progress*0.75}}/>)}
  <View style={{position:"absolute",left:24,top:320,width:Math.floor(ctx.progress*590),height:8,backgroundColor:"#fff"}}/>
</Scene>; }
