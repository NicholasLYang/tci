
typedef char bool;
#define true 1
#define false 0

typedef struct {
  void *next;
  char *bump;
  uint64_t len;
} BumpList;

// if ptr != NULL, then ptr is the aligned bump pointer value to return, and
// next_bump is the next value of the bump pointer
typedef struct {
  void *ptr;
  void *next_bump;
} Bump;

// align must be a power of 2
Bump bump_ptr(void *bump_, void *end, uint64_t size) {
  char *bump = (char *)(((((size_t)bump_ - 1) >> 3) + 1) << 3);
  Bump result = {NULL, NULL};
  result.next_bump = bump + size;
  if (result.next_bump > end) {
    result.next_bump = NULL;
  } else
    result.ptr = bump;

  return result;
}

void *bump_alloc(BumpList *bump, uint64_t size) {
  char *array_begin = (char *)(bump + 1), *bucket_end = array_begin + bump->len;

  Bump result = bump_ptr(bump->bump, bucket_end, size);
  if (result.ptr != NULL) {
    bump->bump = result.next_bump;
    return result.ptr;
  }

  if (bump->next != NULL)
    return bump_alloc(bump->next, size);

  uint64_t next_len = bump->len / 2 + bump->len;
  if (next_len < size)
    next_len = size;

  bump->next = malloc(sizeof(*bump) + next_len);

  BumpList *next = bump->next;
  next->len = next_len;
  next->next = NULL;
  char *ptr = (char *)(next + 1);
  next->bump = ptr + size;

  return ptr;
}

BumpList *bump_new(void) {
  BumpList *bump = malloc(sizeof(BumpList) + 1024);
  bump->next = NULL;
  bump->bump = (char *)(bump + 1);
  bump->len = 1024;
  return bump;
}

typedef struct {
  char *str;
  uint64_t len;
} String;

char CHAR_ARRAY[1024];

void char_array_finalize(char **arr) { dyn_array_add(arr, '\0'); }

uint64_t char_array_add_string(char **arr, String str) {
  return dyn_array_add_from(arr, str.str, str.len);
}

String string_new(char *str) {
  String string = {str, strlen(str)};
  return string;
}

String string_from_parts(char *str, uint64_t len) {
  String s = {str, len};
  return s;
}

char *read_file(char *name) {
  FILE *file = fopen(name, "r");
  if (file == NULL)
    return NULL;

  dyn_array_declare(arr, char);

  char buf[256];
  size_t char_count;
  while ((char_count = fread(buf, 1, 256, file))) {
    dyn_array_add_from(&arr, buf, char_count);
  }
  fclose(file);
  char_array_finalize(&arr);
  return arr;
}

bool streq(String s, char *val) {
  size_t len = strlen(val);
  if (s.len != len)
    return false;

  for (size_t i = 0; i < len; i++)
    if (s.str[i] != val[i])
      return false;

  return true;
}

String t_itoa(uint32_t val) {
  uint32_t length = snprintf(NULL, 0, "%u", val);
  snprintf(CHAR_ARRAY, length + 1, "%u", val);
  return string_from_parts(CHAR_ARRAY, length);
}