use anyhow::{bail, Context, Result};
use glam::{Vec2, Vec3};
use lazy_static::lazy_static;
use regex::Regex;

#[derive(Debug, PartialEq)]
#[repr(C)]
pub struct ObjVertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
}

#[derive(Debug, PartialEq)]
enum ObjLine {
    Position(Vec3),
    Normal(Vec3),
    UV(Vec2),
    Face([(u32, u32, u32); 3]),
    Object(String),
    Material(String),
    Comment(String),
    SmoothShading(String),
    Group(String),
}

pub fn parse_obj<'a, I>(lines: I) -> Result<(Vec<ObjVertex>, Vec<u32>)>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut positions = Vec::<Vec3>::new();
    let mut normals = Vec::<Vec3>::new();
    let mut uvs = Vec::<Vec2>::new();

    let mut vertices = Vec::<ObjVertex>::new();
    let mut indices = Vec::<u32>::new();

    for line in lines.into_iter() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed = parse_line(line).context("Invalid line")?;
        match parsed {
            ObjLine::Position(pos) => positions.push(pos),
            ObjLine::Normal(normal) => normals.push(normal),
            ObjLine::UV(uv) => uvs.push(uv),
            ObjLine::Face(verts) => verts.iter().for_each(|(p, t, n)| {
                vertices.push(ObjVertex {
                    position: positions[(p - 1) as usize],
                    normal: normals[(n - 1) as usize],
                    uv: uvs[(t - 1) as usize],
                });
                indices.push(vertices.len() as u32 - 1);
            }),
            ObjLine::Comment(_)
            | ObjLine::Object(_)
            | ObjLine::Material(_)
            | ObjLine::SmoothShading(_)
            | ObjLine::Group(_) => (),
        }
    }

    Ok((vertices, indices))
}

fn parse_line(line: &str) -> Result<ObjLine> {
    lazy_static! {
        static ref POSITION_RE: Regex =
            Regex::new(r"^v\s*(-?\d*\.?\d*)\s*(-?\d*\.?\d*)\s(-?\d*\.?\d*)").unwrap();
        static ref NORMAL_RE: Regex =
            Regex::new(r"^vn\s*(-?\d*\.?\d*)\s*(-?\d*\.?\d*)\s(-?\d*\.?\d*)").unwrap();
        static ref UV_RE: Regex =
            Regex::new(r"^vt\s*(-?\d*\.?\d*)\s*(-?\d*\.?\d*)\s?(-?\d*\.?\d*)?").unwrap();
        static ref MATERIAL_RE: Regex = Regex::new(r"^usemtl\s*(.*)").unwrap();
        static ref OBJECT_RE: Regex = Regex::new(r"^o\s*(.*)").unwrap();
        static ref GROUP_RE: Regex = Regex::new(r"^g\s*(.*)").unwrap();
        static ref SMOOTHSHADING_RE: Regex = Regex::new(r"^s\s*(.*)").unwrap();
        static ref FACES_RE: Regex =
            Regex::new(r"^f\s+(\d*)?/(\d*)?/(\d*)?\s+(\d*)?/(\d*)?/(\d*)?\s+(\d*)?/(\d*)?/(\d*)?")
                .unwrap();
    }

    if let Some(captures) = POSITION_RE.captures(line) {
        return Ok(ObjLine::Position(Vec3::new(
            captures
                .get(1)
                .context("Not enough matches")?
                .as_str()
                .parse::<f32>()?,
            captures
                .get(2)
                .context("Not enough matches")?
                .as_str()
                .parse::<f32>()?,
            captures
                .get(3)
                .context("Not enough matches")?
                .as_str()
                .parse::<f32>()?,
        )));
    }

    if let Some(captures) = NORMAL_RE.captures(line) {
        return Ok(ObjLine::Normal(Vec3::new(
            captures
                .get(1)
                .context("Not enough matches")?
                .as_str()
                .parse::<f32>()?,
            captures
                .get(2)
                .context("Not enough matches")?
                .as_str()
                .parse::<f32>()?,
            captures
                .get(3)
                .context("Not enough matches")?
                .as_str()
                .parse::<f32>()?,
        )));
    }

    if let Some(captures) = UV_RE.captures(line) {
        return Ok(ObjLine::UV(Vec2::new(
            captures
                .get(1)
                .context("Not enough matches")?
                .as_str()
                .parse::<f32>()?,
            captures
                .get(2)
                .context("Not enough matches")?
                .as_str()
                .parse::<f32>()?,
        )));
    }

    if let Some(captures) = FACES_RE.captures(line) {
        return Ok(ObjLine::Face([
            (
                captures
                    .get(1)
                    .context("Not enough matches")?
                    .as_str()
                    .parse::<u32>()?,
                captures
                    .get(2)
                    .context("Not enough matches")?
                    .as_str()
                    .parse::<u32>()?,
                captures
                    .get(3)
                    .context("Not enough matches")?
                    .as_str()
                    .parse::<u32>()?,
            ),
            (
                captures
                    .get(4)
                    .context("Not enough matches")?
                    .as_str()
                    .parse::<u32>()?,
                captures
                    .get(5)
                    .context("Not enough matches")?
                    .as_str()
                    .parse::<u32>()?,
                captures
                    .get(6)
                    .context("Not enough matches")?
                    .as_str()
                    .parse::<u32>()?,
            ),
            (
                captures
                    .get(7)
                    .context("Not enough matches")?
                    .as_str()
                    .parse::<u32>()?,
                captures
                    .get(8)
                    .context("Not enough matches")?
                    .as_str()
                    .parse::<u32>()?,
                captures
                    .get(9)
                    .context("Not enough matches")?
                    .as_str()
                    .parse::<u32>()?,
            ),
        ]));
    }

    if let Some(comment) = line.strip_prefix('#') {
        return Ok(ObjLine::Comment(comment.trim().to_string()));
    }

    if let Some(captures) = MATERIAL_RE.captures(line) {
        return Ok(ObjLine::Material(
            captures
                .get(1)
                .context("Not enough captures")?
                .as_str()
                .to_string(),
        ));
    }

    if let Some(captures) = OBJECT_RE.captures(line) {
        return Ok(ObjLine::Object(
            captures
                .get(1)
                .context("Not enough captures")?
                .as_str()
                .to_string(),
        ));
    }

    if let Some(captures) = GROUP_RE.captures(line) {
        return Ok(ObjLine::Group(
            captures
                .get(1)
                .context("Not enough captures")?
                .as_str()
                .to_string(),
        ));
    }

    if let Some(captures) = SMOOTHSHADING_RE.captures(line) {
        return Ok(ObjLine::SmoothShading(
            captures
                .get(1)
                .context("Not enough captures")?
                .as_str()
                .to_string(),
        ));
    }

    bail!("Unknown line encountered:\n{}\n", line);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_comment() {
        let parsed = parse_line("# object mesh").unwrap();

        assert_eq!(parsed, ObjLine::Comment("object mesh".to_string()));
    }

    #[test]
    fn parse_position() {
        let parsed = parse_line("v  -0.6301 1.4997 -0.5411").unwrap();

        assert_eq!(
            parsed,
            ObjLine::Position(Vec3::new(-0.6301, 1.4997, -0.5411))
        );
    }

    #[test]
    fn parse_normal() {
        let parsed = parse_line("vn -0.2165 -0.7775 -0.5904").unwrap();

        assert_eq!(
            parsed,
            ObjLine::Normal(Vec3::new(-0.2165, -0.7775, -0.5904))
        );
    }

    #[test]
    fn parse_uv_3() {
        let parsed = parse_line("vt 0.2536 0.7157 0.0000").unwrap();

        assert_eq!(parsed, ObjLine::UV(Vec2::new(0.2536, 0.7157)));
    }

    #[test]
    fn parse_uv_2() {
        let parsed = parse_line("vt 0.2536 0.7157").unwrap();

        assert_eq!(parsed, ObjLine::UV(Vec2::new(0.2536, 0.7157)));
    }

    #[test]
    fn parse_face() {
        let parsed = parse_line("f 71901/72071/71892 71954/72128/71945 71953/72127/71944").unwrap();

        assert_eq!(
            parsed,
            ObjLine::Face([
                (71901, 72071, 71892),
                (71954, 72128, 71945),
                (71953, 72127, 71944)
            ])
        );
    }

    #[test]
    fn parse_object() {
        let parsed = parse_line("o Japanese_Shrine_Cylinder.030").unwrap();

        assert_eq!(
            parsed,
            ObjLine::Object("Japanese_Shrine_Cylinder.030".to_string())
        );
    }

    #[test]
    fn parse_material() {
        let parsed = parse_line("usemtl Japanese_Shrine_Mat_NONE").unwrap();

        assert_eq!(
            parsed,
            ObjLine::Material("Japanese_Shrine_Mat_NONE".to_string())
        );
    }

    #[test]
    fn parse_group() {
        let parsed = parse_line("g mesh").unwrap();

        assert_eq!(parsed, ObjLine::Group("mesh".to_string()));
    }

    #[test]
    fn parse_smooth_shading_on() {
        let parsed = parse_line("s 1").unwrap();

        assert_eq!(parsed, ObjLine::SmoothShading("1".to_string()));
    }

    #[test]
    fn parse_smooth_shading_off() {
        let parsed = parse_line("s off").unwrap();

        assert_eq!(parsed, ObjLine::SmoothShading("off".to_string()));
    }

    #[test]
    fn parse_simple_obj() {
        let obj_file = "# Blender v2.93.0 OBJ File: ''
# www.blender.org
o Cube
v 0.500000 1.000000 -1.000000
v 0.000000 -1.000000 -1.000000
v 1.000000 -1.000000 -1.000000
vt 0.875000 0.500000
vt 0.625000 0.750000
vt 0.625000 0.500000
vn 0.0000 0.0000 1.0000
vn 0.0000 0.0000 1.0000
vn 0.0000 0.0000 1.0000
s off
f 1/1/1 2/2/2 3/3/3"
            .to_string();

        let (vertices, indices) = parse_obj(obj_file.lines()).unwrap();

        assert_eq!(
            vec![
                ObjVertex {
                    position: Vec3::new(0.5, 1.0, -1.0),
                    normal: Vec3::new(0.0, 0.0, 1.0),
                    uv: Vec2::new(0.875, 0.5)
                },
                ObjVertex {
                    position: Vec3::new(0.0, -1.0, -1.0),
                    normal: Vec3::new(0.0, 0.0, 1.0),
                    uv: Vec2::new(0.625, 0.75)
                },
                ObjVertex {
                    position: Vec3::new(1.0, -1.0, -1.0),
                    normal: Vec3::new(0.0, 0.0, 1.0),
                    uv: Vec2::new(0.625, 0.5)
                },
            ],
            vertices
        );
        assert_eq!(vec![0, 1, 2], indices);
    }
}
