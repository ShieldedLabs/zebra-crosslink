/// An alternative to `[T; K]` that works with const trait params
#[derive(Debug)]
pub struct FixedLengthVec<T, const K: usize>(Vec<T>);

impl<T, const K: usize> TryFrom<Vec<T>> for FixedLengthVec<T, K> {
    type Error = usize;

    fn try_from(v: Vec<T>) -> Result<Self, Self::Error> {
        if v.len() == K {
            Ok(Self(v))
        } else {
            Err(v.len())
        }
    }
}
