//! vanilla density 气候样条 — clean-room 对齐 26.1
//! `util/CubicSpline.java` 的 Multipoint 求值语义与
//! `net/minecraft/data/worldgen/TerrainProvider.java` 的样条外壳。
//!
//! 求值语义锚点（CubicSpline.Multipoint.apply）：
//! - 区间内 t = (input - x1)/(x2 - x1)，
//!   值 = lerp(t, y1, y2) + t·(1-t)·lerp(t, a, b)，
//!   其中 a = d1·(x2-x1) - (y2-y1)，b = -d2·(x2-x1) + (y2-y1)；
//! - 区间外按端点导数线性延伸（d==0 为端点常量钳制）；
//! - 节点值本身可以是另一条样条（嵌套坐标求值）。

/// density 气候输入（每列一组；与 NoiseRouter 的气候通道一一对应）。
#[derive(Clone, Copy, Debug, Default)]
pub struct Climate {
    pub continents: f64,
    pub erosion: f64,
    /// RIDGES：未折叠 weirdness（26.1 overworldFactor 的子样条坐标）。
    pub ridges: f64,
    /// RIDGES_FOLDED = peaksAndValleys(weirdness)。
    pub ridges_folded: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Coord {
    Continents,
    Erosion,
    Ridges,
    RidgesFolded,
}

impl Coord {
    fn get(&self, c: &Climate) -> f64 {
        match self {
            Self::Continents => c.continents,
            Self::Erosion => c.erosion,
            Self::Ridges => c.ridges,
            Self::RidgesFolded => c.ridges_folded,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Spline {
    Constant(f64),
    Multipoint {
        coord: Coord,
        locations: Vec<f64>,
        values: Vec<Spline>,
        derivatives: Vec<f64>,
    },
}

pub struct Builder {
    coord: Coord,
    locations: Vec<f64>,
    values: Vec<Spline>,
    derivatives: Vec<f64>,
}

impl Builder {
    #[must_use]
    pub const fn new(coord: Coord) -> Self {
        Self {
            coord,
            locations: Vec::new(),
            values: Vec::new(),
            derivatives: Vec::new(),
        }
    }

    /// 节点：位置 + 常量值（可选导数）。
    pub fn point(&mut self, location: f64, value: f64) -> &mut Self {
        self.point_d(location, value, 0.0)
    }

    pub fn point_d(&mut self, location: f64, value: f64, derivative: f64) -> &mut Self {
        self.locations.push(location);
        self.values.push(Spline::Constant(value));
        self.derivatives.push(derivative);
        self
    }

    /// 节点：位置 + 嵌套样条值（导数为 0）。
    pub fn spline(&mut self, location: f64, sampler: Spline) -> &mut Self {
        self.locations.push(location);
        self.values.push(sampler);
        self.derivatives.push(0.0);
        self
    }

    #[must_use]
    pub fn build(self) -> Spline {
        Spline::Multipoint {
            coord: self.coord,
            locations: self.locations,
            values: self.values,
            derivatives: self.derivatives,
        }
    }
}

impl Spline {
    #[must_use]
    pub fn eval(&self, c: &Climate) -> f64 {
        match self {
            Self::Constant(v) => *v,
            Self::Multipoint {
                coord,
                locations,
                values,
                derivatives,
            } => {
                let input = coord.get(c);
                let start = find_interval_start(locations, input);
                let last = locations.len() - 1;
                if start < 0 {
                    return linear_extend(input, locations, values[0].eval(c), derivatives, 0);
                }
                if start == last {
                    return linear_extend(
                        input,
                        locations,
                        values[last].eval(c),
                        derivatives,
                        last,
                    );
                }
                let x1 = locations[start];
                let x2 = locations[start + 1];
                let t = (input - x1) / (x2 - x1);
                let y1 = values[start].eval(c);
                let y2 = values[start + 1].eval(c);
                let d1 = derivatives[start];
                let d2 = derivatives[start + 1];
                let a = d1 * (x2 - x1) - (y2 - y1);
                let b = -d2 * (x2 - x1) + (y2 - y1);
                let lt = y1 + (y2 - y1) * t;
                lt + t * (1.0 - t) * (a + (b - a) * t)
            }
        }
    }
}

/// 等价 Mth.binarySearch(0, len, i -> input < locations[i]) - 1：
/// 返回最后一个 input >= locations[i] 的下标（-1 表示 input < 全部节点）。
#[must_use]
fn find_interval_start(locations: &[f64], input: f64) -> i64 {
    let mut lo = 0usize;
    let mut hi = locations.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if input < locations[mid] {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo as i64 - 1
}

#[must_use]
fn linear_extend(
    input: f64,
    locations: &[f64],
    value: f64,
    derivatives: &[f64],
    index: usize,
) -> f64 {
    let derivative = derivatives[index];
    if derivative == 0.0 {
        value
    } else {
        value + derivative * (input - locations[index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 单点区间内 Hermite：等值于三节点的区间插值。
    #[test]
    fn multipoint_interpolates_inside_interval() {
        let s = Builder::new(Coord::Continents)
            .point_d(-1.0, 0.0, 1.0)
            .point(0.0, 1.0)
            .point(1.0, 2.0)
            .build();
        let mut c = Climate::default();
        c.continents = 0.5;
        // t=0.5：lerp=1.5 + 0.25*(a+(b-a)*0.5)，a = 1*1-(1-0)=0，b = -(2-1)= -1
        // 值 = 1.5 + 0.25*(-0.5) = 1.375
        assert!((s.eval(&c) - 1.375).abs() < 1e-12);
    }

    /// 区间外按端点值/导数延伸。
    #[test]
    fn multipoint_clamps_outside_domain() {
        let s = Builder::new(Coord::Erosion)
            .point_d(-1.0, 1.0, 2.0)
            .point(1.0, 3.0)
            .build();
        let mut c = Climate::default();
        c.erosion = -2.0;
        assert!((s.eval(&c) - (1.0 + 2.0 * (-2.0 - (-1.0)))).abs() < 1e-12);
        c.erosion = 5.0;
        assert!((s.eval(&c) - 3.0).abs() < 1e-12);
    }

    /// 嵌套坐标：外层节点值是内层样条。
    #[test]
    fn nested_coordinates_evaluate() {
        let inner = Builder::new(Coord::RidgesFolded)
            .point(-1.0, 0.5)
            .point(1.0, 1.5)
            .build();
        let outer = Builder::new(Coord::Continents).spline(0.0, inner).build();
        let mut c = Climate::default();
        c.continents = 0.0;
        c.ridges_folded = 0.5;
        assert!((outer.eval(&c) - 1.0).abs() < 1e-12);
    }
}
