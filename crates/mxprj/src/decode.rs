//! Schema-driven decoding of the document archive into a generic [`Value`] tree.
//!
//! Each C++ type the archive contains has a loader (`fn(&mut Decoder, version)`)
//! registered under its C++ name in [`crate::types`]. Loaders mirror the type's
//! `serialize`/`load` function (decompiled listings: `tools/ghidra/decompile.sh`).
//! The decoder handles boost's framing: class preambles, object ids, pointers and
//! back-references.
//!
//! The same loaders also write: [`encode`] runs them in write mode, where each
//! `int`/`string`/`object`/... call takes the next field of the tree being written
//! (checking its name) and writes it, the way boost's `serialize` is symmetric.
//! Decoding the output gives the same tree back (object ids renumbered in order).

use crate::archive::{Header, Reader, Session, Slot, WSession, Writer};
use crate::container::{Error, Result};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    Null,
    /// Back-reference to the object with this id.
    Ref(u64),
    List(Vec<Value>),
    Obj(Obj),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Obj {
    pub type_name: String,
    pub version: u32,
    pub oid: Option<u64>,
    /// The export name this object's class was written under, when it was the first
    /// object of its class reached through a polymorphic pointer (it can be a legacy
    /// alias of `type_name`).
    pub export: Option<String>,
    pub fields: Vec<(String, Value)>,
}

impl Obj {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// Like `get`, but also searches merged base objects (`<Base>` fields).
    pub fn find(&self, name: &str) -> Option<&Value> {
        self.get(name).or_else(|| {
            self.fields.iter().filter(|(n, _)| n.starts_with('<')).find_map(|(_, v)| match v {
                Value::Obj(o) => o.find(name),
                _ => None,
            })
        })
    }
}

/// Where a value sits in a tree: per level, the index of the field (in an object)
/// or of the item (in a list).
pub type Path = Vec<u32>;

impl Obj {
    /// Every object at or below this one, with its path from here, depth first.
    pub fn walk_paths<'v>(&'v self, f: &mut impl FnMut(&'v Obj, &[u32])) {
        let mut path = Vec::new();
        Value::walk_obj_paths(self, &mut path, f);
    }

    /// Path (from this object) of the field `find` would return.
    pub fn find_path(&self, name: &str) -> Option<Path> {
        if let Some(i) = self.fields.iter().position(|(n, _)| n == name) {
            return Some(vec![i as u32]);
        }
        self.fields.iter().enumerate().filter(|(_, (n, _))| n.starts_with('<')).find_map(|(i, (_, v))| match v {
            Value::Obj(o) => o.find_path(name).map(|mut p| {
                p.insert(0, i as u32);
                p
            }),
            _ => None,
        })
    }

    pub fn at(&self, path: &[u32]) -> Option<&Value> {
        let (first, rest) = path.split_first()?;
        self.fields.get(*first as usize)?.1.at(rest)
    }

    pub fn at_mut(&mut self, path: &[u32]) -> Option<&mut Value> {
        let (first, rest) = path.split_first()?;
        self.fields.get_mut(*first as usize)?.1.at_mut(rest)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Value> {
        self.fields.iter_mut().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// Like `find`, for changing the value.
    pub fn find_mut(&mut self, name: &str) -> Option<&mut Value> {
        if let Some(i) = self.fields.iter().position(|(n, _)| n == name) {
            return Some(&mut self.fields[i].1);
        }
        self.fields.iter_mut().filter(|(n, _)| n.starts_with('<')).find_map(|(_, v)| match v {
            Value::Obj(o) => o.find_mut(name),
            _ => None,
        })
    }
}

impl Value {
    /// The value at `path` below this one (`[]` is this value).
    pub fn at(&self, path: &[u32]) -> Option<&Value> {
        let Some((first, rest)) = path.split_first() else { return Some(self) };
        match self {
            Value::Obj(o) => o.fields.get(*first as usize)?.1.at(rest),
            Value::List(items) => items.get(*first as usize)?.at(rest),
            _ => None,
        }
    }

    pub fn at_mut(&mut self, path: &[u32]) -> Option<&mut Value> {
        let Some((first, rest)) = path.split_first() else { return Some(self) };
        match self {
            Value::Obj(o) => o.fields.get_mut(*first as usize)?.1.at_mut(rest),
            Value::List(items) => items.get_mut(*first as usize)?.at_mut(rest),
            _ => None,
        }
    }

    fn walk_obj_paths<'v>(o: &'v Obj, path: &mut Vec<u32>, f: &mut impl FnMut(&'v Obj, &[u32])) {
        f(o, path);
        for (i, (_, v)) in o.fields.iter().enumerate() {
            path.push(i as u32);
            v.walk_value_paths(path, f);
            path.pop();
        }
    }

    fn walk_value_paths<'v>(&'v self, path: &mut Vec<u32>, f: &mut impl FnMut(&'v Obj, &[u32])) {
        match self {
            Value::Obj(o) => Value::walk_obj_paths(o, path, f),
            Value::List(items) => {
                for (i, v) in items.iter().enumerate() {
                    path.push(i as u32);
                    v.walk_value_paths(path, f);
                    path.pop();
                }
            }
            _ => {}
        }
    }

    /// Visit every object in the tree, depth first, for changing it.
    pub fn walk_mut(&mut self, f: &mut impl FnMut(&mut Obj)) {
        match self {
            Value::Obj(o) => {
                f(o);
                for (_, v) in &mut o.fields {
                    v.walk_mut(f);
                }
            }
            Value::List(items) => items.iter_mut().for_each(|v| v.walk_mut(f)),
            _ => {}
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Indented text, one field per line.
    pub fn write_tree(&self, out: &mut String, name: &str, depth: usize) {
        use std::fmt::Write;
        let pad = "  ".repeat(depth);
        match self {
            Value::Int(i) => writeln!(out, "{pad}{name}: {i}").unwrap(),
            Value::Float(f) => writeln!(out, "{pad}{name}: {f}").unwrap(),
            Value::Str(s) => writeln!(out, "{pad}{name}: {s:?}").unwrap(),
            Value::Bytes(b) => writeln!(out, "{pad}{name}: <{} bytes>", b.len()).unwrap(),
            Value::Null => writeln!(out, "{pad}{name}: null").unwrap(),
            Value::Ref(oid) => writeln!(out, "{pad}{name}: -> #{oid}").unwrap(),
            Value::List(items) => {
                writeln!(out, "{pad}{name}: [{}]", items.len()).unwrap();
                for (i, v) in items.iter().enumerate() {
                    v.write_tree(out, &format!("[{i}]"), depth + 1);
                }
            }
            Value::Obj(o) => {
                let oid = o.oid.map_or(String::new(), |i| format!(" #{i}"));
                let ty = if o.type_name.is_empty() { String::new() } else { format!(" {} v{}", o.type_name, o.version) };
                writeln!(out, "{pad}{name}:{ty}{oid}").unwrap();
                for (n, v) in &o.fields {
                    v.write_tree(out, n, depth + 1);
                }
            }
        }
    }

    /// Visit every object in the tree, depth first.
    pub fn walk<'v>(&'v self, f: &mut impl FnMut(&'v Obj)) {
        match self {
            Value::Obj(o) => {
                f(o);
                for (_, v) in &o.fields {
                    v.walk(f);
                }
            }
            Value::List(items) => items.iter().for_each(|v| v.walk(f)),
            _ => {}
        }
    }
}

pub type Loader = Rc<dyn Fn(&mut Decoder, u32) -> Result<()>>;

/// Write mode: the output and what is left to write of each object being written.
struct Out {
    w: Writer,
    s: WSession,
    /// Parallel to `Decoder::stack`: the fields still to be written.
    src: Vec<Src>,
    /// The whole tree, to find objects that a pointer refers to before they are
    /// written (as boost does, they are then written there, and referred to where
    /// they would have been).
    tree: Option<Obj>,
    /// Objects (old ids) written ahead like that.
    ahead: std::collections::HashSet<u64>,
}

/// Fields of the tree still to write for one object, list item or group.
enum Src {
    Fields(std::iter::Peekable<std::vec::IntoIter<(String, Value)>>),
    /// A list item or group whose shape is known at the first field: decoding
    /// collapses a node with a single field into that field's value.
    Pending(Value),
}

pub struct Decoder<'a> {
    pub r: Reader<'a>,
    s: Session,
    out: Option<Out>,
    /// Blank mode (see [`blank`]): nothing is read; every value is its default.
    blank: Option<Blank<'a>>,
    loaders: &'a HashMap<String, Loader>,
    /// Export name (as written for polymorphic pointers) -> registered type name.
    exports: &'a HashMap<String, String>,
    /// Objects under construction; the innermost receives fields.
    stack: Vec<Obj>,
    /// Breadcrumb of type names, for errors.
    path: Vec<String>,
    /// From the archive header; changes how collections are framed.
    pub library_version: u64,
}

impl<'a> Decoder<'a> {
    pub fn new(r: Reader<'a>, library_version: u64, loaders: &'a HashMap<String, Loader>, exports: &'a HashMap<String, String>) -> Self {
        Decoder { r, s: Session::new(), out: None, blank: None, loaders, exports, stack: Vec::new(), path: Vec::new(), library_version }
    }

    /// A decoder in write mode (see [`encode`]).
    fn writer(library_version: u64, loaders: &'a HashMap<String, Loader>, exports: &'a HashMap<String, String>) -> Self {
        let out = Out { w: Writer::new(), s: WSession::new(), src: Vec::new(), tree: None, ahead: Default::default() };
        Decoder {
            r: Reader::new(&[], 0),
            s: Session::new(),
            out: Some(out),
            blank: None,
            loaders,
            exports,
            stack: Vec::new(),
            path: Vec::new(),
            library_version,
        }
    }

    pub fn writing(&self) -> bool {
        self.out.is_some()
    }

    /// Building a blank object (see [`blank`]) rather than reading or writing.
    pub fn is_blank(&self) -> bool {
        self.blank.is_some()
    }

    /// An integer that must be `value` in a blank object (a fixed count, say);
    /// otherwise read or written as [`Decoder::int`].
    pub fn int_or_blank(&mut self, name: &str, value: i64) -> Result<i64> {
        if self.is_blank() {
            self.put(name, Value::Int(value));
            return Ok(value);
        }
        self.int(name)
    }

    /// Blank mode: a new object of `type_name`, its version and tracking as the class
    /// table says (version 0, untracked, for classes it doesn't list).
    fn blank_object(&mut self, type_name: &str) -> Result<Obj> {
        let b = self.blank.as_mut().expect("blank mode");
        let (version, tracked) = b.classes.get(type_name).copied().unwrap_or((0, false));
        let oid = tracked.then(|| {
            b.next_oid += 1;
            b.next_oid - 1
        });
        self.run(type_name, version, oid)
    }

    /// Position for errors: in the archive when reading, in the output when writing.
    pub fn offset(&self) -> usize {
        match &self.out {
            Some(o) => o.w.len(),
            None => self.r.offset(),
        }
    }

    /// Bytes left to read; unbounded when writing (for loaders' sanity checks).
    pub fn remaining(&self) -> usize {
        match &self.out {
            Some(_) => usize::MAX,
            None if self.blank.is_some() => usize::MAX,
            None => self.r.remaining(),
        }
    }

    // ---- write mode: taking fields from the tree -------------------------------

    fn push_src(&mut self, src: Src) {
        self.out.as_mut().expect("write mode").src.push(src);
    }

    /// Pops the innermost source; it must be used up.
    fn pop_src(&mut self) -> Result<()> {
        let at = self.offset();
        let rest = match self.out.as_mut().expect("write mode").src.pop() {
            Some(Src::Fields(mut it)) => it.next().map(|(n, _)| n),
            Some(Src::Pending(Value::Obj(o))) if o.type_name.is_empty() && o.fields.is_empty() => None,
            Some(Src::Pending(_)) => Some("(value)".into()),
            None => None,
        };
        match rest {
            None => Ok(()),
            Some(_) => Err(Error { offset: at, what: "tree has fields the schema does not write" }),
        }
    }

    /// Whether the innermost source's next field is called `name`.
    fn next_is(&mut self, name: &str) -> bool {
        match self.out.as_mut().and_then(|o| o.src.last_mut()) {
            Some(Src::Fields(it)) => it.peek().is_some_and(|(n, _)| n == name),
            _ => false,
        }
    }

    /// The next field of the innermost source, which must be called `name`.
    fn take(&mut self, name: &str) -> Result<Value> {
        let at = self.offset();
        let out = self.out.as_mut().expect("write mode");
        let src = out.src.last_mut().ok_or(Error { offset: at, what: "no source to write from" })?;
        if let Src::Pending(_) = src {
            let Src::Pending(v) = std::mem::replace(src, Src::Fields(Vec::new().into_iter().peekable())) else { unreachable!() };
            let fields = match v {
                Value::Obj(o) if o.type_name.is_empty() && o.fields.len() != 1 && o.fields.first().is_some_and(|(n, _)| n == name) => {
                    o.fields
                }
                v => vec![(name.to_string(), v)],
            };
            *src = Src::Fields(fields.into_iter().peekable());
        }
        let Src::Fields(it) = src else { unreachable!() };
        match it.next() {
            Some((n, v)) if n == name => Ok(v),
            Some(_) => Err(Error { offset: at, what: "tree field name does not match the schema" }),
            None => Err(Error { offset: at, what: "tree is missing a field the schema writes" }),
        }
    }

    fn w(&mut self) -> &mut Writer {
        &mut self.out.as_mut().expect("write mode").w
    }

    /// Run `type_name`'s loader as the archive root (a non-pointer object).
    pub fn root(&mut self, type_name: &str) -> Result<Obj> {
        self.stack.push(Obj::default());
        let res = self.object("root", type_name);
        let mut holder = self.stack.pop().unwrap();
        res?;
        match holder.fields.pop() {
            Some((_, Value::Obj(o))) => Ok(o),
            _ => Err(self.err("root was not a new object")),
        }
    }

    pub fn err(&self, what: &'static str) -> Error {
        Error { offset: self.offset(), what }
    }

    /// Where we are, e.g. `Project > ObjectVector<Song> > Song`.
    pub fn path(&self) -> String {
        self.path.join(" > ")
    }

    fn put(&mut self, name: &str, v: Value) {
        if let Some(top) = self.stack.last_mut() {
            top.fields.push((name.to_string(), v));
        }
    }

    // ---- primitives -------------------------------------------------------

    pub fn int(&mut self, name: &str) -> Result<i64> {
        if let Some(b) = &self.blank {
            let v = (b.ints)(&self.path, name).unwrap_or(0);
            self.put(name, Value::Int(v));
            return Ok(v);
        }
        if self.writing() {
            let v = match self.take(name)? {
                Value::Int(v) => v,
                _ => return Err(self.err("expected an integer in the tree")),
            };
            self.w().int(v);
            self.put(name, Value::Int(v));
            return Ok(v);
        }
        let v = self.r.int()?;
        self.put(name, Value::Int(v));
        Ok(v)
    }

    pub fn string(&mut self, name: &str) -> Result<String> {
        if self.is_blank() {
            self.put(name, Value::Str(String::new()));
            return Ok(String::new());
        }
        if self.writing() {
            let v = match self.take(name)? {
                Value::Str(v) => v,
                _ => return Err(self.err("expected a string in the tree")),
            };
            self.w().string(&v);
            self.put(name, Value::Str(v.clone()));
            return Ok(v);
        }
        let v = self.r.string()?;
        self.put(name, Value::Str(v.clone()));
        Ok(v)
    }

    pub fn f32(&mut self, name: &str) -> Result<f32> {
        if self.is_blank() {
            self.put(name, Value::Float(0.0));
            return Ok(0.0);
        }
        if self.writing() {
            let v = self.take_float(name)? as f32;
            self.w().f32(v);
            self.put(name, Value::Float(v as f64));
            return Ok(v);
        }
        let v = self.r.f32()?;
        self.put(name, Value::Float(v as f64));
        Ok(v)
    }

    pub fn f64(&mut self, name: &str) -> Result<f64> {
        if self.is_blank() {
            self.put(name, Value::Float(0.0));
            return Ok(0.0);
        }
        if self.writing() {
            let v = self.take_float(name)?;
            self.w().f64(v);
            self.put(name, Value::Float(v));
            return Ok(v);
        }
        let v = self.r.f64()?;
        self.put(name, Value::Float(v));
        Ok(v)
    }

    /// `n` raw bytes (an opaque blob).
    pub fn bytes(&mut self, name: &str, n: usize) -> Result<Vec<u8>> {
        if self.is_blank() {
            self.put(name, Value::Bytes(vec![0; n]));
            return Ok(vec![0; n]);
        }
        if self.writing() {
            let v = match self.take(name)? {
                Value::Bytes(v) if v.len() == n => v,
                _ => return Err(self.err("expected a blob of the stated length in the tree")),
            };
            self.w().bytes(&v);
            self.put(name, Value::Bytes(v.clone()));
            return Ok(v);
        }
        let v = self.r.bytes(n)?.to_vec();
        self.put(name, Value::Bytes(v.clone()));
        Ok(v)
    }

    /// Raw single byte (`bool`/`char` loaded via load_binary).
    pub fn byte(&mut self, name: &str) -> Result<u8> {
        if self.is_blank() {
            self.put(name, Value::Int(0));
            return Ok(0);
        }
        if self.writing() {
            let v = match self.take(name)? {
                Value::Int(v) if (0..=255).contains(&v) => v as u8,
                _ => return Err(self.err("expected a byte in the tree")),
            };
            self.w().u8(v);
            self.put(name, Value::Int(v as i64));
            return Ok(v);
        }
        let v = self.r.u8()?;
        self.put(name, Value::Int(v as i64));
        Ok(v)
    }

    fn take_float(&mut self, name: &str) -> Result<f64> {
        match self.take(name)? {
            Value::Float(v) => Ok(v),
            Value::Int(v) => Ok(v as f64),
            _ => Err(self.err("expected a number in the tree")),
        }
    }

    // ---- objects ----------------------------------------------------------

    fn run(&mut self, type_name: &str, version: u32, oid: Option<u64>) -> Result<Obj> {
        let loader = self.loaders.get(type_name).cloned().ok_or_else(|| {
            self.path.push(type_name.to_string());
            self.err("no loader for type")
        })?;
        self.stack.push(Obj { type_name: type_name.to_string(), version, oid, ..Obj::default() });
        self.path.push(type_name.to_string());
        let res = (*loader)(self, version);
        let obj = self.stack.pop().unwrap();
        res?;
        self.path.pop();
        Ok(obj)
    }

    /// Write mode: run `type_name`'s loader over the fields of `o`, giving the
    /// object as decoding would build it (with its new id).
    fn run_from(&mut self, type_name: &str, o: Obj, new_oid: Option<u64>) -> Result<Obj> {
        self.push_src(Src::Fields(o.fields.into_iter().peekable()));
        let res = self.run(type_name, o.version, new_oid);
        let done = self.pop_src();
        let mut obj = res?;
        done?;
        obj.export = o.export;
        Ok(obj)
    }

    fn expect_obj(&self, v: Value) -> Result<Obj> {
        match v {
            Value::Obj(o) => Ok(o),
            _ => Err(self.err("expected an object in the tree")),
        }
    }

    /// `ar >> member` for a class with a preamble (the common case).
    pub fn object(&mut self, name: &str, type_name: &str) -> Result<()> {
        if self.is_blank() {
            let o = self.blank_object(type_name)?;
            self.put(name, Value::Obj(o));
            return Ok(());
        }
        if self.writing() {
            let v = match self.take(name)? {
                Value::Ref(old) if !self.out.as_ref().unwrap().s.written(old) => {
                    // Its first place comes later (something was written ahead of
                    // where it was): write it here, and refer to it there.
                    let at = self.offset();
                    let out = self.out.as_mut().unwrap();
                    let o = out.tree.as_ref().and_then(|t| find_oid(t, old)).cloned();
                    let o = o.ok_or(Error { offset: at, what: "back-reference to an object not in the tree" })?;
                    out.ahead.insert(old);
                    let oid = out.s.object_new(&mut out.w, type_name, o.version, o.oid)?;
                    Value::Obj(self.run_from(type_name, o, oid)?)
                }
                Value::Ref(old) => {
                    let out = self.out.as_mut().unwrap();
                    Value::Ref(out.s.object_ref(&mut out.w, type_name, old)?)
                }
                Value::Obj(o) if o.oid.is_some_and(|id| self.out.as_ref().unwrap().ahead.contains(&id)) => {
                    let out = self.out.as_mut().unwrap();
                    Value::Ref(out.s.object_ref(&mut out.w, type_name, o.oid.unwrap())?)
                }
                v => {
                    let o = self.expect_obj(v)?;
                    let out = self.out.as_mut().unwrap();
                    let oid = out.s.object_new(&mut out.w, type_name, o.version, o.oid)?;
                    Value::Obj(self.run_from(type_name, o, oid)?)
                }
            };
            self.put(name, v);
            return Ok(());
        }
        let v = match self.s.object(&mut self.r, type_name)? {
            Slot::New { version, oid, .. } => Value::Obj(self.run(type_name, version, oid)?),
            Slot::Ref(oid) => Value::Ref(oid),
            Slot::Null => Value::Null,
        };
        self.put(name, v);
        Ok(())
    }

    /// `ar >> base_object<Base>(*this)`: the base's fields are merged into the current object.
    pub fn base(&mut self, type_name: &str) -> Result<()> {
        let field = format!("<{type_name}>");
        let obj = if self.is_blank() {
            self.blank_object(type_name)?
        } else if self.writing() {
            let v = self.take(&field)?;
            let o = self.expect_obj(v)?;
            let out = self.out.as_mut().unwrap();
            let oid = out.s.object_new(&mut out.w, type_name, o.version, o.oid)?;
            self.run_from(type_name, o, oid)?
        } else {
            match self.s.object(&mut self.r, type_name)? {
                Slot::New { version, oid, .. } => self.run(type_name, version, oid)?,
                _ => return Err(self.err("base object was a back-reference")),
            }
        };
        if let Some(top) = self.stack.last_mut() {
            top.fields.push((field, Value::Obj(obj)));
        }
        Ok(())
    }

    /// Inline a type that has no preamble (implementation level `object_serializable`).
    /// boost still assigns such a class an id on first load, so register it.
    pub fn inline(&mut self, name: &str, type_name: &str) -> Result<()> {
        let v = if self.is_blank() {
            Value::Obj(self.run(type_name, 0, None)?)
        } else if self.writing() {
            self.out.as_mut().unwrap().s.register(type_name);
            let v = self.take(name)?;
            let o = self.expect_obj(v)?;
            Value::Obj(self.run_from(type_name, o, None)?)
        } else {
            self.s.register(type_name);
            Value::Obj(self.run(type_name, 0, None)?)
        };
        self.put(name, v);
        Ok(())
    }

    /// A pointer (`T*`, `shared_ptr<T>`, `scoped_ptr<T>` payload) declared as
    /// `type_name`. `abstract_` must be set for abstract declared classes, which
    /// boost does not pre-register.
    pub fn pointer(&mut self, name: &str, type_name: &str, abstract_: bool) -> Result<()> {
        if self.is_blank() {
            // A smart pointer's object is there (a blank one of the declared class),
            // unless it is a weak pointer's or the class is abstract; other pointers
            // are null.
            let n = self.path.len();
            let owner = n.checked_sub(1).map(|i| self.path[i].as_str()).unwrap_or("");
            let outer = n.checked_sub(2).map(|i| self.path[i].as_str()).unwrap_or("");
            let owned = name == "px"
                && !abstract_
                && (owner.starts_with("std::shared_ptr<") || owner.starts_with("boost::scoped_ptr<"))
                && !outer.starts_with("std::weak_ptr<");
            let v = if owned { Value::Obj(self.blank_object(type_name)?) } else { Value::Null };
            self.put(name, v);
            return Ok(());
        }
        if self.writing() {
            let v = match self.take(name)? {
                Value::Null => {
                    let out = self.out.as_mut().unwrap();
                    out.s.pointer_null(&mut out.w, type_name, abstract_);
                    Value::Null
                }
                Value::Ref(old) if self.out.as_ref().unwrap().s.written(old) => {
                    let out = self.out.as_mut().unwrap();
                    Value::Ref(out.s.pointer_ref(&mut out.w, type_name, abstract_, old)?)
                }
                Value::Ref(old) => {
                    // Not written yet: write it here.
                    let at = self.offset();
                    let out = self.out.as_mut().unwrap();
                    let o = out.tree.as_ref().and_then(|t| find_oid(t, old)).cloned();
                    let o = o.ok_or(Error { offset: at, what: "back-reference to an object not in the tree" })?;
                    out.ahead.insert(old);
                    self.write_pointee(type_name, abstract_, o)?
                }
                Value::Obj(o) if o.oid.is_some_and(|id| self.out.as_ref().unwrap().ahead.contains(&id)) => {
                    // Written ahead: refer to it.
                    let out = self.out.as_mut().unwrap();
                    Value::Ref(out.s.pointer_ref(&mut out.w, type_name, abstract_, o.oid.unwrap())?)
                }
                v => {
                    let o = self.expect_obj(v)?;
                    self.write_pointee(type_name, abstract_, o)?
                }
            };
            self.put(name, v);
            return Ok(());
        }
        self.pointer_read(name, type_name, abstract_)
    }

    /// Write mode: a new object behind a pointer.
    fn write_pointee(&mut self, type_name: &str, abstract_: bool, o: Obj) -> Result<Value> {
        let actual = o.type_name.clone();
        // The export name: as read, else one that maps to this class.
        let export = o.export.clone().unwrap_or_else(|| {
            let mut names: Vec<&String> = self.exports.iter().filter(|(_, t)| **t == actual).map(|(e, _)| e).collect();
            names.sort();
            names.iter().find(|e| **e == &actual).or(names.first()).map_or(actual.clone(), |e| e.to_string())
        });
        let first = !self.out.as_ref().unwrap().s.is_registered(&actual) && actual != type_name;
        let out = self.out.as_mut().unwrap();
        let oid = out.s.pointer_new(&mut out.w, type_name, abstract_, &actual, &export, o.version, o.oid)?;
        let mut obj = self.run_from(&actual, o, oid)?;
        if !first {
            obj.export = None;
        } else if obj.export.is_none() {
            obj.export = Some(export);
        }
        Ok(Value::Obj(obj))
    }

    fn pointer_read(&mut self, name: &str, type_name: &str, abstract_: bool) -> Result<()> {
        let exports = self.exports;
        let resolve = |n: &str| exports.get(n).cloned().unwrap_or_else(|| n.to_string());
        let v = match self.s.pointer(&mut self.r, type_name, abstract_, &resolve)? {
            Slot::Null => Value::Null,
            Slot::Ref(oid) => Value::Ref(oid),
            Slot::New { version, oid, class_id, class } => {
                let actual = self.s.class_name(class_id).to_string();
                let mut obj = self.run(&actual, version, oid)?;
                obj.export = class;
                Value::Obj(obj)
            }
        };
        self.put(name, v);
        Ok(())
    }

    /// The most recent field called `name` in the object being decoded.
    pub fn field(&self, name: &str) -> Option<&Value> {
        self.stack.last()?.fields.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// Collect the fields read by `f` into a list element / named sub-node. A node
    /// with a single field is stored as that field's value.
    pub fn group<F: FnOnce(&mut Self) -> Result<()>>(&mut self, name: &str, f: F) -> Result<()> {
        if self.writing() {
            let v = self.take(name)?;
            self.push_src(Src::Pending(v));
        }
        self.stack.push(Obj::default());
        let res = f(self);
        let obj = self.stack.pop().unwrap();
        let done = if self.writing() { self.pop_src() } else { Ok(()) };
        res?;
        done?;
        self.put(name, collapse(obj));
        Ok(())
    }

    /// boost collection: count, item version (library version > 3), then `count`
    /// items each read by `item`. Stored as the list of items; an item version other
    /// than 0 is kept in a `<name>#item_version` field just before it.
    pub fn collection<F: FnMut(&mut Self) -> Result<()>>(&mut self, name: &str, mut item: F) -> Result<()> {
        let version_field = format!("{name}#item_version");
        let (values, item_version, count) = if let Some(b) = &self.blank {
            // Empty, with the item version a std::vector of a known class writes.
            let item = self.path.last().and_then(|t| t.strip_prefix("std::vector<")).and_then(|r| r.strip_suffix('>'));
            (None, item.and_then(|x| b.classes.get(x)).map_or(0, |c| c.0 as u64), 0)
        } else if self.writing() {
            let item_version = if self.next_is(&version_field) {
                match self.take(&version_field)? {
                    Value::Int(i) => i as u64,
                    _ => return Err(self.err("expected an item version in the tree")),
                }
            } else {
                0
            };
            let values = match self.take(name)? {
                Value::List(items) => items,
                _ => return Err(self.err("expected a collection in the tree")),
            };
            let n = values.len() as u64;
            self.w().int(n as i64);
            if self.library_version > 3 {
                self.w().int(item_version as i64);
            }
            (Some(values), item_version, n)
        } else {
            let at = self.r.offset();
            let count = self.r.uint()?;
            if count > self.r.remaining() as u64 {
                return Err(Error { offset: at, what: "collection count larger than archive" });
            }
            let item_version = if self.library_version > 3 { self.r.uint()? } else { 0 };
            (None, item_version, count)
        };
        if item_version != 0 {
            self.put(&version_field, Value::Int(item_version as i64));
        }
        let mut sources = values.map(|v| v.into_iter());
        let mut items = Vec::with_capacity(count.min(1 << 16) as usize);
        for _ in 0..count {
            if let Some(src) = sources.as_mut() {
                let v = src.next().unwrap();
                self.push_src(Src::Pending(v));
            }
            self.stack.push(Obj::default());
            let res = item(self);
            let o = self.stack.pop().unwrap();
            let done = if sources.is_some() { self.pop_src() } else { Ok(()) };
            res?;
            done?;
            items.push(collapse(o));
        }
        self.put(name, Value::List(items));
        Ok(())
    }
}

/// A node with exactly one field is stored as that field's value.
fn collapse(o: Obj) -> Value {
    if o.fields.len() == 1 { o.fields.into_iter().next().unwrap().1 } else { Value::Obj(o) }
}

/// Write `root` (a tree as [`Decoder::root`] returns it, of type `type_name`) as a
/// complete archive, header included. Returns the bytes and the tree as decoding
/// them would give it.
pub fn encode(
    root: Obj,
    type_name: &str,
    header: Header,
    loaders: &HashMap<String, Loader>,
    exports: &HashMap<String, String>,
) -> std::result::Result<(Vec<u8>, Obj), (Error, String)> {
    let mut d = Decoder::writer(header.library_version, loaders, exports);
    d.w().header(header);
    d.out.as_mut().unwrap().tree = Some(root.clone());
    d.push_src(Src::Fields(vec![("root".to_string(), Value::Obj(root))].into_iter().peekable()));
    let obj = d.root(type_name).map_err(|e| (e, d.path()))?;
    let out = d.out.take().unwrap();
    Ok((out.w.buf, obj))
}

/// Blank mode's state: the class table, the ids handed out, and integers that are
/// not 0.
struct Blank<'a> {
    classes: &'a HashMap<String, (u32, bool)>,
    next_oid: u64,
    ints: &'a dyn Fn(&[String], &str) -> Option<i64>,
}

/// How to build a blank object (see [`blank`]).
pub struct BlankSpec<'a> {
    /// Each class's version and whether its objects are tracked (have ids); a class
    /// not listed is version 0, untracked.
    pub classes: &'a HashMap<String, (u32, bool)>,
    /// The first object id to hand out.
    pub first_oid: u64,
    /// Integers that are not 0, by the breadcrumb of classes being built (innermost
    /// last) and the field name: flags that decide what follows them.
    pub ints: &'a dyn Fn(&[String], &str) -> Option<i64>,
    /// Build it as part of a whole project (some classes store more there).
    pub project_level: bool,
}

/// A new object of `type_name` as its loader would read one whose every stored value
/// is a default: 0 (or what `spec.ints` says), 0.0, "" and zero bytes, empty
/// collections, null pointers except smart pointers' objects, and member objects
/// built the same way. Returns the object and the next free object id.
pub fn blank(
    type_name: &str,
    spec: &BlankSpec,
    library_version: u64,
    loaders: &HashMap<String, Loader>,
    exports: &HashMap<String, String>,
) -> std::result::Result<(Obj, u64), (Error, String)> {
    let mut d = Decoder::new(Reader::new(&[], 0), library_version, loaders, exports);
    d.blank = Some(Blank { classes: spec.classes, next_oid: spec.first_oid, ints: spec.ints });
    if spec.project_level {
        d.path.push("NI::MASCHINE::DATA::Project".into());
    }
    let obj = d.blank_object(type_name).map_err(|e| (e, d.path()))?;
    Ok((obj, d.blank.unwrap().next_oid))
}

/// The object with id `oid` in `o` (depth first).
fn find_oid(o: &Obj, oid: u64) -> Option<&Obj> {
    if o.oid == Some(oid) {
        return Some(o);
    }
    o.fields.iter().find_map(|(_, v)| find_oid_in(v, oid))
}

fn find_oid_in(v: &Value, oid: u64) -> Option<&Obj> {
    match v {
        Value::Obj(o) => find_oid(o, oid),
        Value::List(l) => l.iter().find_map(|v| find_oid_in(v, oid)),
        _ => None,
    }
}
