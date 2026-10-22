export const composition = { width: 640, height: 360, fps: 60, duration: 10 };
export const controls={assets:{sprites:asset({kind:"image",required:true})}};
export default function ImageLayer(){ const time = useCurrentTime();return <Scene style={{width:640,height:360,backgroundColor:"#101010"}}>
  <Image src="asset://sprites" style={{position:"absolute",left:40+Math.floor(time.progress*100),top:70,width:240,height:160,opacity:0.6+time.progress*0.4}}/>
</Scene>;}
