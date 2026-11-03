export const composition = { width: 360, height: 180, fps: 1, duration: 1 };

// Icons as pasted from an icon set: strokes of currentColor on a 24 px grid, round caps and joins,
// without fill; at several sizes and stroke widths; filled ones; and duotone ones, a faint fill
// under a stroke. Each is a component with its shapes as children.
function Icon({ size = 32, weight = 2, color = "#263238", children }) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={weight}
      strokeLinecap="round"
      strokeLinejoin="round"
      style={{ color }}
    >
      {children}
    </svg>
  );
}

function Home() {
  return (
    <>
      <path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
      <polyline points="9 22 9 12 15 12 15 22" />
    </>
  );
}

function Search() {
  return (
    <>
      <circle cx={11} cy={11} r={8} />
      <line x1={21} y1={21} x2={16.65} y2={16.65} />
    </>
  );
}

function Heart() {
  return (
    <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z" />
  );
}

function Sun() {
  return (
    <>
      <circle cx={12} cy={12} r={5} />
      <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
    </>
  );
}

function Star(props) {
  return (
    <polygon
      points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"
      {...props}
    />
  );
}

function Bell() {
  return (
    <>
      <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" />
      <path d="M13.73 21a2 2 0 0 1-3.46 0" />
    </>
  );
}

export default function Scene() {
  const row = { display: "flex", alignItems: "center", gap: 10, marginBottom: 10 };
  return (
    <View style={{ padding: 10 }}>
      <View style={row}>
        <Icon>
          <Home />
        </Icon>
        <Icon>
          <Search />
        </Icon>
        <Icon>
          <Heart />
        </Icon>
        <Icon>
          <Sun />
        </Icon>
        <Icon>
          <Star />
        </Icon>
        <Icon>
          <Bell />
        </Icon>
      </View>
      <View style={row}>
        <Icon size={16} color="#1e88e5">
          <Home />
        </Icon>
        <Icon size={24} weight={1.5} color="#43a047">
          <Search />
        </Icon>
        <Icon size={40} weight={1} color="#e53935">
          <Heart />
        </Icon>
        <Icon size={56} weight={0.75} color="#fb8c00">
          <Sun />
        </Icon>
        <Icon size={20} weight={3} color="#8e24aa">
          <Bell />
        </Icon>
      </View>
      <View style={row}>
        <Icon color="#e53935">
          <path
            fill="currentColor"
            stroke="none"
            d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l8.84 8.84 8.84-8.84a5.5 5.5 0 0 0 0-7.78z"
          />
        </Icon>
        <Icon color="#fbc02d">
          <Star fill="currentColor" />
        </Icon>
        <Icon color="#1e88e5">
          <circle cx={12} cy={12} r={9} fill="currentColor" fillOpacity={0.2} />
          <path d="m8 12 3 3 5-6" />
        </Icon>
        <Icon color="#00897b">
          <Star fill="currentColor" fillOpacity={0.25} strokeOpacity={0.8} />
        </Icon>
      </View>
    </View>
  );
}
