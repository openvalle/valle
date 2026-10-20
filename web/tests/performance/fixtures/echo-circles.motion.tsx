export const composition = { width: 640, height: 360, fps: 60, duration: 10 };
export const controls = { props: { shift: number({default:0}) } };
const DOTS = Array.from({length:1024}, (_,i)=>({id:`d${i}`,x:20+(i%32)*19,y:22+Math.floor(i/32)*10,k:i/1024}));
export default function EchoCircles(props) { const ctx = useTime();
  const t=ctx.seconds;
  return <Scene style={{width:640,height:360,backgroundColor:"#101010"}}>
    <Echo key="trail" count={3} interval={3} decay={0.36}>
      {DOTS.map(dot=><Circle key={dot.id}
        cx={dot.x+sin(t*2+dot.k*3)*4+cos(t*3+dot.k*7)*3+props.shift}
        cy={dot.y+sin(t*5+dot.k*11)*2+cos(t*7+dot.k*13)*2}
        r={1.2+0.3*sin(t*4+dot.k*17)} fill="#8b7bff"
        style={{opacity:0.5+0.2*sin(t*6+dot.k*19)}} />)}
    </Echo>
  </Scene>;
}
