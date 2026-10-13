//! The PDF object model (ISO 32000-2 §7.3).

/// An indirect reference, `num gen R`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjRef {
    pub num: u32,
    pub generation: u16,
}

impl ObjRef {
    pub fn new(num: u32, generation: u16) -> Self {
        ObjRef { num, generation }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Object {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    /// Name without the leading slash, `#xx` escapes already decoded.
    Name(Vec<u8>),
    /// String bytes, escapes already decoded (literal and hex strings alike).
    Str(Vec<u8>),
    Array(Vec<Object>),
    Dict(Dict),
    Stream(Stream),
    Ref(ObjRef),
}

/// A dictionary that keeps its keys in file order, so rewritten files stay
/// recognisable when diffed against the original.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dict {
    entries: Vec<(Vec<u8>, Object)>,
}

/// A stream: its dictionary and its still-encoded data.
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub dict: Dict,
    pub data: Vec<u8>,
}

impl Dict {
    pub fn new() -> Self {
        Dict::default()
    }

    pub fn get(&self, key: &[u8]) -> Option<&Object> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &[u8]) -> Option<&mut Object> {
        self.entries
            .iter_mut()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// Insert or replace. A later duplicate key in a file replaces the earlier one.
    pub fn set(&mut self, key: impl Into<Vec<u8>>, value: Object) {
        let key = key.into();
        if let Some(slot) = self.get_mut(&key) {
            *slot = value;
        } else {
            self.entries.push((key, value));
        }
    }

    pub fn remove(&mut self, key: &[u8]) -> Option<Object> {
        let i = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(i).1)
    }

    pub fn contains(&self, key: &[u8]) -> bool {
        self.get(key).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &Object)> {
        self.entries.iter().map(|(k, v)| (k.as_slice(), v))
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Object> {
        self.entries.iter_mut().map(|(_, v)| v)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The value of `key` when it is a name, e.g. `/Type /Page` gives `b"Page"`.
    pub fn get_name(&self, key: &[u8]) -> Option<&[u8]> {
        self.get(key).and_then(Object::as_name)
    }

    pub fn get_int(&self, key: &[u8]) -> Option<i64> {
        self.get(key).and_then(Object::as_int)
    }
}

impl Object {
    pub fn name(s: &str) -> Object {
        Object::Name(s.as_bytes().to_vec())
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Object::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Object::Int(i) => Some(*i as f64),
            Object::Real(r) if r.is_finite() => Some(*r),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            Object::Name(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&[u8]> {
        match self {
            Object::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Object>> {
        match self {
            Object::Array(a) => Some(a),
            _ => None,
        }
    }

    /// The dictionary of a dictionary or of a stream.
    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }

    pub fn as_dict_mut(&mut self) -> Option<&mut Dict> {
        match self {
            Object::Dict(d) => Some(d),
            Object::Stream(s) => Some(&mut s.dict),
            _ => None,
        }
    }

    pub fn as_stream(&self) -> Option<&Stream> {
        match self {
            Object::Stream(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_objref(&self) -> Option<ObjRef> {
        match self {
            Object::Ref(r) => Some(*r),
            _ => None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Object::Null => "null",
            Object::Bool(_) => "boolean",
            Object::Int(_) => "integer",
            Object::Real(_) => "real",
            Object::Name(_) => "name",
            Object::Str(_) => "string",
            Object::Array(_) => "array",
            Object::Dict(_) => "dictionary",
            Object::Stream(_) => "stream",
            Object::Ref(_) => "reference",
        }
    }

    /// Visit every `Object::Ref` inside this object (the closure may replace it).
    /// Nesting is bounded by the parser's depth limit, so this recursion is too.
    pub fn for_each_ref_mut(&mut self, f: &mut dyn FnMut(&mut Object)) {
        match self {
            Object::Ref(_) => f(self),
            Object::Array(a) => a.iter_mut().for_each(|o| o.for_each_ref_mut(f)),
            Object::Dict(d) => d.values_mut().for_each(|o| o.for_each_ref_mut(f)),
            Object::Stream(s) => s.dict.values_mut().for_each(|o| o.for_each_ref_mut(f)),
            _ => {}
        }
    }

    /// Collect every reference inside this object.
    pub fn collect_refs(&self, out: &mut Vec<ObjRef>) {
        match self {
            Object::Ref(r) => out.push(*r),
            Object::Array(a) => a.iter().for_each(|o| o.collect_refs(out)),
            Object::Dict(d) => d.iter().for_each(|(_, o)| o.collect_refs(out)),
            Object::Stream(s) => s.dict.iter().for_each(|(_, o)| o.collect_refs(out)),
            _ => {}
        }
    }
}

impl From<Dict> for Object {
    fn from(d: Dict) -> Self {
        Object::Dict(d)
    }
}

impl From<ObjRef> for Object {
    fn from(r: ObjRef) -> Self {
        Object::Ref(r)
    }
}
